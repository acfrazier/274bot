//! Shared live-harness ownership and adapter boundary for the panel and TUI.
//!
//! This module is available only with `live-harness`. It owns scenario and
//! fixture adapter values so the UI crates can keep only surface state.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

/// An owned scenario. The scenario adapter value stays private to this crate.
pub struct Scenario(Box<scenario::Scenario>);

/// A borrowed, scalar view of the scenario settings used by the front ends.
#[derive(Debug, Clone, Copy)]
pub struct ScenarioView<'a> {
    name: &'a str,
    profile_count: usize,
    seed_profiles: &'a [(&'static str, &'static str)],
    mainland: bool,
    renderer: bool,
    only_render_selected: bool,
    capture: bool,
    full_rate: bool,
    deadline: Duration,
    terminal_shot: Option<&'a str>,
    start_script: Option<&'a str>,
    start_file: Option<&'a str>,
    fleet_start: Option<FleetStart>,
    wait_script_stop: Option<&'a str>,
    inject_companion_as: Option<&'a str>,
    companion_count: usize,
    has_start_script_step: bool,
    nav: ScenarioNav,
}

/// Surface-independent view settings selected by a scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScenarioNav {
    allow_teleports: bool,
    allow_wilderness: bool,
    show_nav_path: bool,
    hop_labels: bool,
    hop_label_px: i32,
    collision_fill: bool,
    nsew_labels: bool,
    client_trail: bool,
    component_flood: bool,
    camera_follow: bool,
    engine_speed_ms: Option<u32>,
}

/// Scenario-owned fleet start mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FleetStart {
    ClueDuel,
    JiveKq,
}

/// The fixture mode accepted by the live prepare path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureMode {
    Default,
    Prepare,
    RunPrepared,
}

/// An owned fixture identity. Its adapter representation remains private.
pub struct FixtureIdentity(Box<scenario::FixtureIdentity>);

/// The isolated account names, credentials, and vault secret for one live boot.
pub struct LiveProfiles {
    names: Vec<String>,
    entries: Vec<(String, String)>,
    passphrase: String,
}

/// A scenario runner owned by frontend-core.
pub struct Runner(Box<scenario::ScenarioRunner>);

/// Shared runner and Start state used by both UI front ends and slot frames.
pub struct RunnerHandle(std::sync::Arc<std::sync::Mutex<RunnerState>>);

pub(crate) struct ScenarioFrame<Tick, Companion> {
    runner: RunnerHandle,
    tick_with_hold: Tick,
    companion_tick: Companion,
}

impl<Tick, Companion> ScenarioFrame<Tick, Companion> {
    pub(crate) fn new(
        runner: RunnerHandle,
        tick_with_hold: Tick,
        companion_tick: Companion,
    ) -> Self {
        Self {
            runner,
            tick_with_hold,
            companion_tick,
        }
    }
}

impl<Client, Tick, Companion> crate::frames::LiveClientFrame<Client>
    for ScenarioFrame<Tick, Companion>
where
    Tick: Fn(&mut scenario::ScenarioRunner, &mut Client, bool) + Send + Sync + 'static,
    Companion: Fn(&mut scenario::ScenarioRunner, usize, &mut Client) + Send + Sync + 'static,
{
    fn run(&self, client: &mut Client, name: &str, hold: bool) {
        self.runner.frame(
            client,
            name,
            hold,
            &self.tick_with_hold,
            &self.companion_tick,
        );
    }
}

#[derive(Default)]
struct RunnerState {
    runner: Option<scenario::ScenarioRunner>,
    pending: Vec<host_play::live_start::PendingCatalogStart>,
    start_handle: Option<host_play::ScriptStartHandle>,
    catalog: Option<host_play::catalog_core::CoreWatch>,
    pair: Option<host_play::paired_core::PairWatch>,
    #[cfg(feature = "js")]
    external: Option<host_play::external_loader::ExternalWatch>,
    #[cfg(feature = "js")]
    external_reload_pending: Option<ExternalReloadPending>,
    live_timing: Option<host_play::live_gate::LiveTiming>,
    core_mode: Option<host_play::live_gate::LiveCore>,
}

#[cfg(feature = "js")]
enum ExternalReloadPending {
    Unchanged,
    Changed {
        path: PathBuf,
        source_before: String,
        source_after: String,
        compiled_before: String,
    },
}

/// Provider-neutral view of the live runner's terminal or progress state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerStatus {
    Seeding,
    Running { step: usize, total: usize },
    Passed,
    Failed,
}

/// Value-only timing receipt for the TUI's live proof output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveTimingView {
    setup_elapsed: Duration,
    script_elapsed: Option<Duration>,
}

impl LiveTimingView {
    pub fn setup_elapsed(&self) -> Duration {
        self.setup_elapsed
    }

    pub fn script_elapsed(&self) -> Option<Duration> {
        self.script_elapsed
    }
}

/// Enabled native witness family for a live scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveCoreMode {
    Off,
    Catalog,
    Pair,
}

/// One surface-neutral result of a shared native proof gate.
pub struct ProofGate {
    kind: ProofGateKind,
    failure: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProofGateKind {
    Disabled,
    Pending,
    Qualified,
    Failed,
}

impl ProofGate {
    pub fn is_disabled(&self) -> bool {
        self.kind == ProofGateKind::Disabled
    }
    pub fn is_pending(&self) -> bool {
        self.kind == ProofGateKind::Pending
    }
    pub fn is_qualified(&self) -> bool {
        self.kind == ProofGateKind::Qualified
    }
    pub fn failure_message(&self) -> Option<&str> {
        self.failure.as_deref()
    }
}

/// Owned gate results and compact proof records for one headed poll.
pub struct WatchDecision {
    catalog: ProofGate,
    pair: ProofGate,
    #[cfg(feature = "js")]
    external: ProofGate,
    catalog_record: Option<String>,
    pair_record: Option<String>,
    #[cfg(feature = "js")]
    external_record: Option<String>,
}

impl WatchDecision {
    pub fn catalog(&self) -> &ProofGate {
        &self.catalog
    }
    pub fn pair(&self) -> &ProofGate {
        &self.pair
    }
    #[cfg(feature = "js")]
    pub fn external(&self) -> &ProofGate {
        &self.external
    }
    pub fn catalog_record(&self) -> Option<&str> {
        self.catalog_record.as_deref()
    }
    pub fn pair_record(&self) -> Option<&str> {
        self.pair_record.as_deref()
    }
    #[cfg(feature = "js")]
    pub fn external_record(&self) -> Option<&str> {
        self.external_record.as_deref()
    }

    /// Decide whether scenario PASS remains held by a gate or clean-stop.
    pub fn pass_hold(
        &self,
        wait_script_stop: Option<&str>,
        stopped: impl FnOnce(&str) -> bool,
        grace_started: &mut Option<Instant>,
        now: Instant,
    ) -> PassHold {
        let pending = self.catalog.is_pending() || self.pair.is_pending();
        #[cfg(feature = "js")]
        let pending = pending || self.external.is_pending();
        if pending {
            return PassHold {
                kind: PassHoldKind::Core,
                failure: None,
                clean_stop: false,
            };
        }
        let Some(needle) = wait_script_stop else {
            return PassHold {
                kind: PassHoldKind::Release,
                failure: None,
                clean_stop: false,
            };
        };
        if stopped(needle) {
            return PassHold {
                kind: PassHoldKind::Release,
                failure: None,
                clean_stop: true,
            };
        }
        let started = *grace_started.get_or_insert(now);
        if now.saturating_duration_since(started) >= host_play::live_gate::SCRIPT_STOP_WAIT {
            PassHold {
                kind: PassHoldKind::TimedOut,
                failure: Some(format!(
                    "timed out waiting for script Idle and clean stop reason {needle:?}"
                )),
                clean_stop: false,
            }
        } else {
            PassHold {
                kind: PassHoldKind::CleanStop,
                failure: None,
                clean_stop: false,
            }
        }
    }

    /// Emit each configured proof record with its stable headed proof tag.
    pub fn for_each_record(&self, mut emit: impl FnMut(&str, &str)) {
        for (tag, record) in [
            ("CATALOG_CORE", self.catalog_record.as_deref()),
            ("PAIRED_CORE", self.pair_record.as_deref()),
            #[cfg(feature = "js")]
            ("EXTERNAL_LOADER", self.external_record.as_deref()),
        ] {
            if let Some(record) = record {
                emit(tag, record);
            }
        }
    }
}

/// Surface-neutral kind of PASS hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassHoldKind {
    Core,
    CleanStop,
    TimedOut,
    Release,
}

/// Opaque PASS-hold result with an optional timeout reason.
pub struct PassHold {
    kind: PassHoldKind,
    failure: Option<String>,
    clean_stop: bool,
}

impl PassHold {
    pub fn kind(&self) -> PassHoldKind {
        self.kind
    }
    pub fn failure_message(&self) -> Option<&str> {
        self.failure.as_deref()
    }
    pub fn clean_stop(&self) -> bool {
        self.clean_stop
    }
}

/// An opaque collection of Starts staged for a live scenario.
pub struct StartPlan(Vec<host_play::live_start::PendingCatalogStart>);

impl StartPlan {
    #[cfg(feature = "js")]
    /// Build Starts for each member of a scenario-owned fleet.
    pub fn fleet_catalog(
        scripts: &mut crate::Scripts,
        fleet: FleetStart,
        names: &[String],
        loadouts: &[api::loadout::Loadout],
    ) -> Result<Self, String> {
        let (library, settings, inject) = scripts.live_start_parts();
        let fleet = match fleet {
            FleetStart::ClueDuel => scenario::FleetStart::ClueDuel,
            FleetStart::JiveKq => scenario::FleetStart::JiveKq,
        };
        host_play::live_start::fleet_catalog_starts(
            fleet, names, library, settings, inject, loadouts,
        )
        .map(Self)
    }

    #[cfg(feature = "js")]
    /// Build one loaded-JS Start, including an optional scenario companion.
    pub fn catalog_card(
        scripts: &mut crate::Scripts,
        names: &[String],
        source: crate::scripts::CardSource,
        lookup: &str,
        bag: Option<Map<String, Value>>,
        inject_companion_as: Option<&str>,
        loadouts: Vec<api::loadout::Loadout>,
    ) -> Result<Self, String> {
        let (library, _, _) = scripts.live_start_parts();
        let script_source = source.to_script();
        let card = library
            .get(script_source, lookup)
            .cloned()
            .ok_or_else(|| format!("loaded script {lookup} is unavailable"))?;
        let siblings = host_play::live_start::card_siblings(library, &card)?;
        let mut starts = vec![host_play::live_start::PendingCatalogStart::load(
            names[0].clone(),
            card.js.clone(),
            card.shape,
            bag.clone(),
            siblings.clone(),
            loadouts.clone(),
        )];
        if let Some(key) = inject_companion_as {
            if names.len() > 1 {
                let mut companion_bag = bag.unwrap_or_default();
                companion_bag.insert(key.to_string(), Value::String(screen_name(&names[0])));
                starts.push(host_play::live_start::PendingCatalogStart::load(
                    names[1].clone(),
                    card.js,
                    card.shape,
                    Some(companion_bag),
                    siblings,
                    loadouts,
                ));
            }
        }
        Ok(Self(starts))
    }

    /// Build the compiled-card Start used by a live scenario.
    pub fn compiled(
        slot: impl Into<String>,
        id: crate::scripts::CompiledCardId,
        bag: Map<String, Value>,
    ) -> Self {
        Self(vec![host_play::live_start::PendingCatalogStart::compiled(
            slot.into(),
            id.to_script(),
            bag,
        )])
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Runner {
    /// Wrap a scenario runner a test built and drove itself.
    pub(crate) fn from_scenario_runner(runner: scenario::ScenarioRunner) -> Self {
        Self(Box::new(runner))
    }

    /// The wrapped scenario runner, for test fixtures that drive it.
    pub(crate) fn scenario_runner_mut(&mut self) -> &mut scenario::ScenarioRunner {
        &mut self.0
    }
}

impl Runner {
    /// Start a scenario runner over the selected world.
    pub(crate) fn with_world(
        scenario: Scenario,
        world: Option<std::sync::Arc<nav_planner::world::NavWorld>>,
    ) -> Self {
        Self(Box::new(scenario::ScenarioRunner::with_world(
            *scenario.0,
            world,
        )))
    }

    /// Set the runner's live profile names.
    pub(crate) fn set_live_names(&mut self, names: &[String]) {
        self.0.set_live_names(names);
    }

    /// Set the runner's scenario deadline.
    pub(crate) fn set_deadline(&mut self, deadline: Duration) {
        self.0.set_deadline(deadline);
    }

    /// Copy the current live endpoint and Play-derived runner inputs.
    pub(crate) fn configure_for_play<Io>(
        &mut self,
        session: &crate::OperatorSession<Io>,
        host: &str,
        port: u16,
    ) {
        self.0.set_tick_speed_endpoint(host, port);
        if let Some(play) = session.play() {
            self.0.set_map_members(play.map_members());
            self.0.set_obj_names(play.obj_names());
        }
    }
}

impl Default for RunnerHandle {
    fn default() -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new(
            RunnerState::default(),
        )))
    }
}

impl RunnerHandle {
    /// Install a new scenario runner.
    pub fn install(&self, runner: Runner) {
        self.0.lock().unwrap().runner = Some(*runner.0);
    }

    /// Install an adapter runner for a front-end integration test.
    #[cfg(feature = "test-support")]
    pub fn install_scenario_runner_for_test(&self, runner: scenario::ScenarioRunner) {
        self.0.lock().unwrap().runner = Some(runner);
    }

    /// Set a terminal capture label on the installed runner in a test.
    #[cfg(feature = "test-support")]
    pub fn set_terminal_shot_for_test(&self, label: &'static str) {
        if let Some(runner) = self.0.lock().unwrap().runner.as_mut() {
            runner.set_terminal_shot(label);
        }
    }

    /// Install a native catalog witness for a front-end integration test.
    #[cfg(feature = "test-support")]
    pub fn install_catalog_watch_for_test(
        &self,
        watch: Option<host_play::catalog_core::CoreWatch>,
    ) {
        self.0.lock().unwrap().catalog = watch;
    }

    /// Install a native pair witness for a front-end integration test.
    #[cfg(feature = "test-support")]
    pub fn install_pair_watch_for_test(&self, watch: Option<host_play::paired_core::PairWatch>) {
        self.0.lock().unwrap().pair = watch;
    }

    #[cfg(feature = "js")]
    /// Install an external-loader witness for a front-end integration test.
    #[cfg(feature = "test-support")]
    pub fn install_external_watch_for_test(
        &self,
        watch: Option<host_play::external_loader::ExternalWatch>,
    ) {
        self.0.lock().unwrap().external = watch;
    }

    /// Remove the current scenario runner, staged Starts, timing, and watches.
    pub fn clear(&self) {
        let mut state = self.0.lock().unwrap();
        state.runner.take();
        state.pending.clear();
        state.start_handle = None;
        state.catalog = None;
        state.pair = None;
        #[cfg(feature = "js")]
        {
            state.external = None;
            state.external_reload_pending = None;
        }
        state.live_timing = None;
        state.core_mode = None;
    }

    /// Replace the pending scenario Starts.
    pub fn stage(&self, plan: StartPlan) {
        self.0.lock().unwrap().pending = plan.0;
    }

    /// Copy staged adapter Starts for a front-end integration test.
    #[cfg(feature = "test-support")]
    pub fn pending_starts_for_test(&self) -> Vec<host_play::live_start::PendingCatalogStart> {
        self.0.lock().unwrap().pending.clone()
    }

    #[cfg(feature = "js")]
    /// Resolve a loaded card and stage its normal or pair-witness Starts.
    pub fn stage_loaded_card(
        &self,
        scripts: &mut crate::Scripts,
        names: &[String],
        choice: &crate::scripts::ScriptChoice,
        bag: Option<Map<String, Value>>,
        inject_companion_as: Option<&str>,
        loadouts: Vec<api::loadout::Loadout>,
    ) -> Result<(), String> {
        let (source, lookup) = choice
            .loaded_card()
            .ok_or_else(|| "live start requires a loaded card choice".to_string())?;
        let pair = {
            let state = self.0.lock().unwrap();
            match state.core_mode {
                Some(host_play::live_gate::LiveCore::Pair(case)) => Some((
                    case,
                    state
                        .pair
                        .clone()
                        .ok_or_else(|| "pair core watch handle unavailable".to_string())?,
                )),
                _ => None,
            }
        };
        let plan = if let Some((case, watch)) = pair {
            let (library, _, _) = scripts.live_start_parts();
            let card = library
                .get(source.to_script(), lookup)
                .cloned()
                .ok_or_else(|| format!("loaded script {lookup} is unavailable"))?;
            let siblings = host_play::live_start::card_siblings(library, &card)?;
            StartPlan(host_play::live_start::pair_starts(
                names,
                case,
                card.js,
                card.shape,
                &card.settings_schema,
                siblings,
                &watch,
            )?)
        } else {
            StartPlan::catalog_card(
                scripts,
                names,
                source,
                lookup,
                bag,
                inject_companion_as,
                loadouts,
            )?
        };
        self.stage(plan);
        Ok(())
    }

    /// Whether a scenario runner is installed.
    pub fn is_present(&self) -> bool {
        self.0.lock().unwrap().runner.is_some()
    }

    /// Validate and retain the core-witness selection for this run.
    pub fn resolve_core(
        &self,
        scenario_name: &str,
        profile_count: usize,
        catalog_enabled: bool,
        pair_enabled: bool,
    ) -> Result<LiveCoreMode, String> {
        let mode = host_play::live_gate::LiveCore::resolve(
            scenario_name,
            profile_count,
            catalog_enabled,
            pair_enabled,
        )?;
        let view = match mode {
            host_play::live_gate::LiveCore::Off => LiveCoreMode::Off,
            host_play::live_gate::LiveCore::Catalog(_) => LiveCoreMode::Catalog,
            host_play::live_gate::LiveCore::Pair(_) => LiveCoreMode::Pair,
        };
        self.0.lock().unwrap().core_mode = Some(mode);
        Ok(view)
    }

    pub(crate) fn configure_core(
        &self,
        play: &host_play::Play,
        names: &[String],
    ) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        let mode = state
            .core_mode
            .ok_or_else(|| "live core mode was not resolved".to_string())?;
        mode.configure(play, names)?;
        state.catalog = Some(play.catalog_core_watch());
        state.pair = Some(play.paired_core_watch());
        Ok(())
    }

    /// Compute the shared witness deadline for a run.
    pub fn core_deadline(
        &self,
        budget: Option<Duration>,
        scenario_deadline: Duration,
        now: Instant,
    ) -> Option<Instant> {
        let state = self.0.lock().unwrap();
        host_play::live_gate::core_deadline(
            state.catalog.as_ref(),
            state.pair.as_ref(),
            budget,
            scenario_deadline,
            now,
        )
    }

    /// Evaluate all configured native watches and produce their terminal records.
    pub fn watch_decision(&self, deadline: Option<Instant>, now: Instant) -> WatchDecision {
        let state = self.0.lock().unwrap();
        let catalog =
            host_play::live_gate::catalog_core_gate(state.catalog.as_ref(), deadline, now);
        let pair = host_play::live_gate::pair_core_gate(state.pair.as_ref(), deadline, now);
        #[cfg(feature = "js")]
        let external = host_play::live_gate::external_core_gate(state.external.as_ref());
        WatchDecision {
            catalog_record: host_play::live_gate::catalog_core_record(
                state.catalog.as_ref(),
                &catalog,
            ),
            pair_record: host_play::live_gate::pair_core_record(state.pair.as_ref(), &pair),
            #[cfg(feature = "js")]
            external_record: host_play::live_gate::external_core_record(
                state.external.as_ref(),
                &external,
            ),
            catalog: proof_gate(catalog),
            pair: proof_gate(pair),
            #[cfg(feature = "js")]
            external: proof_gate(external),
        }
    }

    /// Whether the paired-core witness was configured for this run.
    pub fn pair_core_configured(&self) -> bool {
        self.0
            .lock()
            .unwrap()
            .pair
            .as_ref()
            .is_some_and(host_play::paired_core::PairWatch::configured)
    }
    /// Start the bounded pair-fixture setup clock when this is a configured Flax run.
    pub fn configure_pair_setup_timing(&self, started_at: Instant, core_deadline: Option<Instant>) {
        let mut state = self.0.lock().unwrap();
        let required = matches!(
            state.core_mode,
            Some(host_play::live_gate::LiveCore::Pair(
                host_play::paired_core::PairCase::Flax
            ))
        ) && state
            .pair
            .as_ref()
            .is_some_and(host_play::paired_core::PairWatch::configured);
        state.live_timing =
            required.then(|| host_play::live_gate::LiveTiming::new(started_at, core_deadline));
    }

    /// Whether the bounded paired-fixture setup period has expired.
    pub fn pair_setup_expired(&self, now: Instant) -> bool {
        self.0
            .lock()
            .unwrap()
            .live_timing
            .as_ref()
            .is_some_and(|timing| {
                timing.setup_expired(host_play::live_gate::PAIR_FIXTURE_SETUP_DEADLINE, now)
            })
    }

    /// Copy the timing values needed for a TUI proof receipt.
    pub fn timing_view(&self, now: Instant) -> Option<LiveTimingView> {
        self.0
            .lock()
            .unwrap()
            .live_timing
            .as_ref()
            .map(|timing| LiveTimingView {
                setup_elapsed: timing.setup_elapsed(now),
                script_elapsed: timing.script_elapsed(now),
            })
    }

    /// Whether the catalog-core witness was configured for this run.
    pub fn catalog_core_configured(&self) -> bool {
        self.0
            .lock()
            .unwrap()
            .catalog
            .as_ref()
            .is_some_and(host_play::catalog_core::CoreWatch::configured)
    }

    /// Find a terminal asset-start failure among this runner's owned slots.
    pub(crate) fn terminal_startup_failure(
        &self,
        statuses: &[host_play::SlotStatus],
    ) -> Option<String> {
        let names = self.owned_profile_names();
        statuses.iter().find_map(|status| {
            names
                .iter()
                .any(|name| name.as_str() == status.username.as_str())
                .then(|| {
                    status
                        .terminal_startup_error()
                        .map(|error| format!("{}: {error}", status.username))
                })
                .flatten()
        })
    }

    #[cfg(feature = "js")]
    /// The external-loader account owned by the configured witness.
    pub fn external_loader_account(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap()
            .external
            .as_ref()
            .filter(|watch| watch.configured())
            .map(|watch| watch.account())
    }

    #[cfg(feature = "js")]
    /// Feed the witness its account's newly polled script log lines.
    pub fn note_external_loader_logs(&self, lines: &[(String, String)]) {
        let state = self.0.lock().unwrap();
        let Some(watch) = state.external.as_ref().filter(|watch| watch.configured()) else {
            return;
        };
        let account = watch.account();
        for (slot, line) in lines {
            if account.as_str() == slot.as_str() {
                watch.note_logs(Instant::now(), slot, std::slice::from_ref(line));
            }
        }
    }

    #[cfg(feature = "js")]
    /// Configure the owned external-loader watch for this live account.
    pub fn configure_external_loader(&self, account: String, source: PathBuf) {
        let watch = host_play::external_loader::ExternalWatch::default();
        watch.configure(
            account,
            source,
            host_play::external_loader::FROZEN_SHA256.into(),
        );
        self.0.lock().unwrap().external = Some(watch);
    }

    #[cfg(feature = "js")]
    /// Whether the external-loader witness is configured.
    pub fn external_loader_configured(&self) -> bool {
        self.0
            .lock()
            .unwrap()
            .external
            .as_ref()
            .is_some_and(host_play::external_loader::ExternalWatch::configured)
    }

    #[cfg(feature = "js")]
    pub fn external_loader_cleanup_pending(&self) -> bool {
        self.0
            .lock()
            .unwrap()
            .external
            .as_ref()
            .is_some_and(host_play::external_loader::ExternalWatch::cleanup_pending)
    }

    #[cfg(feature = "js")]
    pub fn external_loader_needs_terminal_hold(&self) -> bool {
        self.0
            .lock()
            .unwrap()
            .external
            .as_ref()
            .is_some_and(host_play::external_loader::ExternalWatch::needs_terminal_hold)
    }

    #[cfg(feature = "js")]
    pub fn external_loader_terminal_capture_due(&self) -> bool {
        self.0
            .lock()
            .unwrap()
            .external
            .as_ref()
            .is_some_and(host_play::external_loader::ExternalWatch::terminal_capture_due)
    }

    #[cfg(feature = "js")]
    pub fn note_external_loader_capture_requested(&self) {
        if let Some(watch) = self.0.lock().unwrap().external.as_ref() {
            watch.note_capture_requested();
        }
    }

    #[cfg(feature = "js")]
    pub fn fail_external_loader(&self, reason: &str) {
        if let Some(watch) = self.0.lock().unwrap().external.as_ref() {
            watch.fail(reason);
        }
    }

    #[cfg(feature = "js")]
    /// Record a terminal prerequisite-runner result for the external proof.
    pub fn note_external_loader_prerequisite(&self) {
        let state = self.0.lock().unwrap();
        let Some(watch) = state.external.as_ref() else {
            return;
        };
        match state.runner.as_ref().map(scenario::ScenarioRunner::status) {
            Some(scenario::RunnerStatus::Passed) => watch.note_prereq_passed(),
            Some(scenario::RunnerStatus::Failed(message)) => watch.note_prereq_failed(message),
            _ => {}
        }
    }

    #[cfg(feature = "js")]
    /// Forward one failed Start for the external proof's owned account.
    pub fn fail_external_loader_start(&self, name: &str, reason: String) {
        if let Some(watch) = self.0.lock().unwrap().external.as_ref() {
            if watch.configured() && watch.account() == name {
                watch.fail_start(reason);
            }
        }
    }

    #[cfg(feature = "js")]
    pub(crate) fn pump_external_loader<Io>(
        &self,
        scripts: &mut crate::Scripts,
        core: &mut crate::OperatorSession<Io>,
        selection: &mut Option<crate::scripts::ScriptChoice>,
        statuses: &crate::StatusBuf,
        now: Instant,
    ) {
        use crate::scripts::{CardSource, ScriptRunState};
        use host_play::external_loader::{Operation, SCRIPT_NAME};

        let mut state = self.0.lock().unwrap();
        let Some(watch) = state.external.clone() else {
            return;
        };
        if !watch.configured() {
            return;
        }
        watch.poll_deadlines(now);
        let account = watch.account();
        if let Some(slot) = statuses.get(&account) {
            watch.note_scene(slot.ingame(), slot.scene_state());
            watch.note_paint(slot.has_script_paint());
        }
        let counts = core.with_slot_snapshot(&account, |snapshot| {
            let bones = snapshot
                .inventory()
                .iter()
                .filter(|item| item.def.id == host_play::external_loader::BONES_ID)
                .map(|item| item.count)
                .sum();
            let prayer = snapshot
                .stats()
                .iter()
                .find(|stat| stat.index == host_play::external_loader::PRAYER_STAT_ID)
                .map(|stat| stat.xp)
                .unwrap_or(0);
            (bones, prayer)
        });
        if let Some((bones, prayer)) = counts {
            watch.note_inventory(now, &account, bones, prayer);
        }
        if watch.status() == host_play::external_loader::ExternalWatchStatus::Failed {
            let script_state = core.script_state(&account);
            let idle = script_state == ScriptRunState::Idle;
            let mut stop_invoked = false;
            if !idle && script_state != ScriptRunState::Stopping {
                core.stop_script(&account);
                stop_invoked = true;
            }
            watch.note_cleanup_progress(now, idle, stop_invoked);
            return;
        }
        match watch.dispatch_operation() {
            Some(Operation::LoadRawTs) => {
                let path = watch.source_path();
                let already_loaded = selection
                    .as_ref()
                    .and_then(crate::scripts::ScriptChoice::loaded_card)
                    .is_some_and(|(source, lookup)| {
                        source == CardSource::File
                            && scripts
                                .card(CardSource::File, lookup)
                                .is_some_and(|card| card.path() == path)
                    });
                if !already_loaded {
                    match scripts.load_file(&path) {
                        Ok(card) => *selection = Some(card.choice()),
                        Err(error) => {
                            watch.fail(format!("load: {error}"));
                            return;
                        }
                    }
                }
                let lookup = selection
                    .as_ref()
                    .and_then(crate::scripts::ScriptChoice::loaded_card)
                    .map_or_else(
                        || path.to_string_lossy().into_owned(),
                        |(_, lookup)| lookup.to_owned(),
                    );
                if let Some(Err(error)) =
                    scripts.take_external_loader_transpile_result(CardSource::File, &lookup)
                {
                    watch.fail(format!("transpile: {error}"));
                    return;
                }
                if !scripts.card_is_prepared(CardSource::File, &lookup) {
                    match scripts.request_external_loader_transpile(CardSource::File, &lookup) {
                        Ok(_) => return,
                        Err(error) => {
                            watch.fail(format!("transpile: {error}"));
                            return;
                        }
                    }
                }
                let Some(card) = scripts
                    .card(CardSource::File, &lookup)
                    .or_else(|| scripts.card(CardSource::File, &path.display().to_string()))
                else {
                    watch.fail("external loader load did not produce a File card");
                    return;
                };
                let count = scripts
                    .cards()
                    .filter(|candidate| {
                        candidate.source() == CardSource::File
                            && (candidate.identity_key() == card.identity_key()
                                || candidate.name() == SCRIPT_NAME)
                    })
                    .count();
                let choice = card.choice();
                let selected = selection.as_ref() == Some(&choice)
                    || scripts.pending_browse(&account) == Some(&choice);
                let running = core.script_state(&account) != ScriptRunState::Idle;
                watch.note_load(
                    count,
                    card.name(),
                    card.path(),
                    &card.identity_key(),
                    card.sha256(),
                    selected,
                    running,
                );
            }
            Some(Operation::Start) => {
                if core.script_start_wait_reason().is_some() || watch.begin_start(now).is_err() {
                    return;
                }
                if let Err(error) = scripts.start_selected(core, &account, selection.as_ref(), None)
                {
                    watch.fail_start(error.to_string());
                }
            }
            Some(Operation::Stop) => {
                let script_state = core.script_state(&account);
                let paint = statuses
                    .get(&account)
                    .is_some_and(|slot| slot.has_script_paint());
                match script_state {
                    ScriptRunState::Idle => watch.note_stop(now, true, paint),
                    ScriptRunState::Stopping => watch.note_stop(now, false, paint),
                    _ => {
                        core.stop_script(&account);
                        watch.note_stop(now, false, paint);
                    }
                }
            }
            Some(Operation::ReloadUnchanged) => {
                match state.external_reload_pending.as_ref() {
                    None => {
                        state.external_reload_pending = Some(ExternalReloadPending::Unchanged);
                        let target =
                            scripts.reload_target(core, Some(&account), selection.as_ref());
                        scripts.begin_reload(core, target);
                    }
                    Some(ExternalReloadPending::Unchanged) => {}
                    Some(ExternalReloadPending::Changed { .. }) => {
                        watch.fail("external loader reload operation changed while pending");
                        return;
                    }
                }
                if scripts.reload_validation_pending() {
                    return;
                }
                let Some(outcome) = scripts.take_reload_outcome() else {
                    return;
                };
                if !matches!(
                    state.external_reload_pending.take(),
                    Some(ExternalReloadPending::Unchanged)
                ) {
                    watch.fail("external loader unchanged reload lost its pending operation");
                    return;
                }
                if outcome.is_nothing_changed() {
                    watch.note_reload_unchanged(host_play::external_loader::NOTHING_CHANGED);
                } else {
                    watch.fail(format!("unchanged reload: {outcome:?}"));
                }
            }
            Some(Operation::ReloadChanged) => {
                if state.external_reload_pending.is_none() {
                    let path = watch.source_path();
                    let before_bytes = match std::fs::read(&path) {
                        Ok(bytes) => bytes,
                        Err(error) => {
                            watch.fail(format!("changed reload read {}: {error}", path.display()));
                            return;
                        }
                    };
                    let source_before = host_play::external_loader::source_sha256(&before_bytes);
                    let compiled_before = scripts
                        .card(CardSource::File, &path.display().to_string())
                        .map(|card| card.sha256().to_owned())
                        .unwrap_or_default();
                    if let Err(error) = host_play::external_loader::apply_harmless_whitespace(&path)
                    {
                        watch.fail(error);
                        return;
                    }
                    let after_bytes = match std::fs::read(&path) {
                        Ok(bytes) => bytes,
                        Err(error) => {
                            watch
                                .fail(format!("changed reload reread {}: {error}", path.display()));
                            return;
                        }
                    };
                    state.external_reload_pending = Some(ExternalReloadPending::Changed {
                        path,
                        source_before,
                        source_after: host_play::external_loader::source_sha256(&after_bytes),
                        compiled_before,
                    });
                    let target = scripts.reload_target(core, Some(&account), selection.as_ref());
                    scripts.begin_reload(core, target);
                }
                if scripts.reload_validation_pending() {
                    return;
                }
                let Some(outcome) = scripts.take_reload_outcome() else {
                    return;
                };
                let Some(ExternalReloadPending::Changed {
                    path,
                    source_before,
                    source_after,
                    compiled_before,
                }) = state.external_reload_pending.take()
                else {
                    watch.fail("external loader changed reload lost its pending operation");
                    return;
                };
                let (applied, nothing_changed) = if outcome.applied_counts().is_some() {
                    (true, false)
                } else if outcome.is_nothing_changed() {
                    (false, true)
                } else if let Some(error) = outcome.failure() {
                    watch.fail(format!("changed reload: {error}"));
                    return;
                } else {
                    watch.fail(format!("changed reload: {outcome:?}"));
                    return;
                };
                let Some(card) = scripts.card(CardSource::File, &path.display().to_string()) else {
                    watch.fail("external loader changed reload lost the File card");
                    return;
                };
                let count = scripts
                    .cards()
                    .filter(|candidate| {
                        candidate.source() == CardSource::File
                            && (candidate.identity_key() == card.identity_key()
                                || candidate.name() == SCRIPT_NAME)
                    })
                    .count();
                let choice = card.choice();
                let selected = selection.as_ref() == Some(&choice)
                    || scripts.pending_browse(&account) == Some(&choice);
                let running = core.script_state(&account) != ScriptRunState::Idle;
                watch.note_reload_changed(
                    count,
                    applied,
                    nothing_changed,
                    card.path(),
                    &card.identity_key(),
                    &source_before,
                    &source_after,
                    &compiled_before,
                    card.sha256(),
                    selected,
                    running,
                );
            }
            _ => {}
        }
    }

    /// Current provider-neutral runner state.
    pub fn status(&self) -> Option<RunnerStatus> {
        self.0
            .lock()
            .unwrap()
            .runner
            .as_ref()
            .map(|runner| match runner.status() {
                scenario::RunnerStatus::Seeding => RunnerStatus::Seeding,
                scenario::RunnerStatus::Running { step, total } => {
                    RunnerStatus::Running { step, total }
                }
                scenario::RunnerStatus::Passed => RunnerStatus::Passed,
                scenario::RunnerStatus::Failed(_) => RunnerStatus::Failed,
            })
    }
    /// Scenario identity used by the headed recording sidecar.
    pub fn scenario_name(&self) -> Option<&'static str> {
        self.0
            .lock()
            .unwrap()
            .runner
            .as_ref()
            .map(scenario::ScenarioRunner::scenario_name)
    }

    /// Runner position and tick count for one recorded frame.
    pub fn record_step(&self) -> Option<(usize, usize, &'static str, u32)> {
        self.0.lock().unwrap().runner.as_ref().map(|runner| {
            let (step, total, name) = runner.record_step();
            (step, total, name, runner.total_ticks())
        })
    }

    #[cfg(feature = "js")]
    /// Exercise the installed Start handle without exposing its adapter type.
    #[cfg(feature = "test-support")]
    pub fn start_native_tick_for_test(
        &self,
        name: &str,
        source: String,
        settings_bag: Option<Map<String, Value>>,
    ) -> Result<(), String> {
        let handle = self
            .0
            .lock()
            .unwrap()
            .start_handle
            .clone()
            .ok_or_else(|| "no live Start handle installed".to_string())?;
        handle.start_load(
            name,
            source,
            script_js::LoadShape::NativeTick,
            settings_bag,
            Vec::new(),
        )
    }

    /// Current scenario failure reason, if the runner failed.
    pub fn failure_message(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap()
            .runner
            .as_ref()
            .and_then(|runner| match runner.status() {
                scenario::RunnerStatus::Failed(message) => Some(message),
                _ => None,
            })
    }

    /// Current terminal evidence JSON, if the scenario has reached a result.
    pub fn evidence_json(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap()
            .runner
            .as_ref()
            .and_then(|runner| runner.evidence().map(|evidence| evidence.to_json()))
    }

    /// Build the stable failure line, preserving the adapter evidence object.
    pub fn failure_record(&self, live_name: &str, message: &str) -> String {
        let state = self.0.lock().unwrap();
        let evidence = state
            .runner
            .as_ref()
            .and_then(scenario::ScenarioRunner::evidence);
        let scenario = evidence
            .map(|evidence| evidence.scenario.clone())
            .unwrap_or_else(|| {
                live_name
                    .strip_prefix("script_")
                    .unwrap_or(live_name)
                    .to_string()
            });
        serde_json::json!({
            "scenario": scenario,
            "outcome": "FAIL",
            "message": message,
            "prerequisite": evidence,
        })
        .to_string()
    }

    /// Current terminal evidence scenario name.
    pub fn evidence_scenario(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap()
            .runner
            .as_ref()
            .and_then(|runner| runner.evidence().map(|evidence| evidence.scenario.clone()))
    }

    /// Install the scenario's headed capture sink.
    pub fn set_shot_sink(
        &self,
        sink: impl FnMut(&str, &api::snapshot::GameSnapshot) + Send + 'static,
    ) {
        if let Some(runner) = self.0.lock().unwrap().runner.as_mut() {
            runner.set_shot_sink(Box::new(sink));
        }
    }

    /// Scenario's terminal capture label.
    pub fn terminal_shot(&self) -> Option<&'static str> {
        self.0
            .lock()
            .unwrap()
            .runner
            .as_ref()
            .and_then(scenario::ScenarioRunner::terminal_shot)
    }

    /// Scenario's optional script to wait for after its proof.
    pub fn wait_script_stop(&self) -> Option<&'static str> {
        self.0
            .lock()
            .unwrap()
            .runner
            .as_ref()
            .and_then(scenario::ScenarioRunner::wait_script_stop)
    }

    /// The name of the scenario's driven profile.
    pub fn profile_name(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap()
            .runner
            .as_ref()
            .map(|runner| runner.profile_name().to_string())
    }

    /// Names of profiles whose startup belongs to this scenario.
    pub fn owned_profile_names(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .runner
            .as_ref()
            .map_or_else(Vec::new, scenario::ScenarioRunner::owned_profile_names)
    }

    /// Scenario companion's profile name by index.
    pub fn companion_profile_name(&self, index: usize) -> Option<String> {
        self.0
            .lock()
            .unwrap()
            .runner
            .as_ref()
            .map(|runner| runner.companion_profile_name(index).to_string())
    }

    /// Find whether this scenario drives the named slot.
    pub fn drives(&self, name: &str) -> bool {
        self.0
            .lock()
            .unwrap()
            .runner
            .as_ref()
            .is_some_and(|runner| runner.drives(name))
    }

    /// Find a scenario companion assigned to the named slot.
    pub fn companion_for(&self, name: &str) -> Option<usize> {
        self.0
            .lock()
            .unwrap()
            .runner
            .as_ref()
            .and_then(|runner| runner.companion_for(name))
    }

    /// Share the runner handle with a per-frame front-end hook.
    pub fn share(&self) -> Self {
        Self(std::sync::Arc::clone(&self.0))
    }

    /// The installed runner's deadline (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn deadline_for_test(&self) -> Option<Duration> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .runner
            .as_ref()
            .map(scenario::ScenarioRunner::deadline)
    }

    /// Hold the runner lock while the guard lives (lock-order tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn lock_for_test(&self) -> impl Drop + '_ {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Borrow the scenario route and aim for one named slot during projection.
    pub fn with_route_for<R>(
        &self,
        name: &str,
        project: impl FnOnce(
            bool,
            Option<&nav_planner::router::Route>,
            Option<api::snapshot::WorldTile>,
        ) -> R,
    ) -> R {
        let state = self.0.lock().unwrap();
        match state.runner.as_ref() {
            Some(runner) if runner.drives(name) => {
                project(true, runner.armed_route(), runner.current_aim())
            }
            _ => project(false, None, None),
        }
    }

    pub(crate) fn install_play(&self, play: &host_play::Play) {
        let mut state = self.0.lock().unwrap();
        state.start_handle = Some(play.script_start_handle());
        state.catalog = Some(play.catalog_core_watch());
        state.pair = Some(play.paired_core_watch());
    }

    pub(crate) fn frame<C, T, U>(
        &self,
        client: &mut C,
        name: &str,
        hold: bool,
        tick_with_hold: T,
        companion_tick: U,
    ) where
        T: Fn(&mut scenario::ScenarioRunner, &mut C, bool),
        U: Fn(&mut scenario::ScenarioRunner, usize, &mut C),
    {
        let mut state = self.0.lock().unwrap();
        let RunnerState {
            runner: runner_slot,
            pending,
            start_handle,
            catalog,
            pair,
            live_timing,
            ..
        } = &mut *state;
        let Some(runner) = runner_slot.as_mut() else {
            return;
        };
        if runner.drives(name) {
            if runner.wants_script_paint() {
                let paint = start_handle.as_ref().and_then(|handle| handle.paint(name));
                if let Some(paint) = paint {
                    runner.observe_script_paint(paint.lines.iter().map(String::as_str));
                }
            }
            let on_start_script = runner.on_start_script();
            let pump = if pair_setup_expired(live_timing) {
                runner.fail_start("paired fixture setup deadline expired before Start");
                host_play::live_start::StartScriptPump::Hold
            } else {
                host_play::live_start::fire_pending_catalog_start(
                    pending,
                    on_start_script,
                    runner.on_stop_script(),
                    || host_play::live_start::StartArming {
                        handle: start_handle.as_ref().cloned(),
                        catalog: catalog.as_ref().cloned(),
                        pair: pair.as_ref().cloned(),
                    },
                )
            };
            mark_live_script_start(live_timing, pending, on_start_script, &pump, Instant::now());
            match pump {
                host_play::live_start::StartScriptPump::Hold => {}
                host_play::live_start::StartScriptPump::Failed(error) => {
                    runner.fail_start(&error);
                }
                host_play::live_start::StartScriptPump::Continue
                | host_play::live_start::StartScriptPump::CompiledRunning => {
                    if pump == host_play::live_start::StartScriptPump::CompiledRunning {
                        runner.observe_script_running();
                    }
                    if runner.on_stop_script() {
                        runner.observe_script_idle(
                            start_handle
                                .as_ref()
                                .is_some_and(|handle| handle.idle(name)),
                        );
                    }
                    tick_with_hold(runner, client, hold);
                    if runner.on_start_script() {
                        let pump = if pair_setup_expired(live_timing) {
                            runner.fail_start("paired fixture setup deadline expired before Start");
                            host_play::live_start::StartScriptPump::Hold
                        } else {
                            host_play::live_start::fire_pending_catalog_start(
                                pending,
                                true,
                                false,
                                || host_play::live_start::StartArming {
                                    handle: start_handle.as_ref().cloned(),
                                    catalog: catalog.as_ref().cloned(),
                                    pair: pair.as_ref().cloned(),
                                },
                            )
                        };
                        mark_live_script_start(live_timing, pending, true, &pump, Instant::now());
                        match pump {
                            host_play::live_start::StartScriptPump::CompiledRunning => {
                                runner.observe_script_running();
                            }
                            host_play::live_start::StartScriptPump::Failed(error) => {
                                runner.fail_start(&error);
                            }
                            _ => {}
                        }
                    }
                }
            }
        } else if let Some(index) = runner.companion_for(name) {
            companion_tick(runner, index, client);
        }
    }
}

fn pair_setup_expired(timing: &Option<host_play::live_gate::LiveTiming>) -> bool {
    timing.as_ref().is_some_and(|timing| {
        timing.setup_expired(
            host_play::live_gate::PAIR_FIXTURE_SETUP_DEADLINE,
            Instant::now(),
        )
    })
}

fn mark_live_script_start(
    timing: &mut Option<host_play::live_gate::LiveTiming>,
    pending: &[host_play::live_start::PendingCatalogStart],
    on_start_script: bool,
    pump: &host_play::live_start::StartScriptPump,
    now: Instant,
) {
    if !on_start_script
        || !matches!(
            pump,
            host_play::live_start::StartScriptPump::Continue
                | host_play::live_start::StartScriptPump::CompiledRunning
        )
        || pending.is_empty()
        || pending.iter().any(|card| !card.started)
    {
        return;
    }
    if let Some(timing) = timing {
        timing.mark_script_started(now);
    }
}

fn proof_gate(gate: host_play::live_gate::CoreGate) -> ProofGate {
    let (kind, failure) = match gate {
        host_play::live_gate::CoreGate::Disabled => (ProofGateKind::Disabled, None),
        host_play::live_gate::CoreGate::Pending => (ProofGateKind::Pending, None),
        host_play::live_gate::CoreGate::Qualified(_) => (ProofGateKind::Qualified, None),
        host_play::live_gate::CoreGate::Failed(message) => (ProofGateKind::Failed, Some(message)),
    };
    ProofGate { kind, failure }
}

impl Scenario {
    /// Resolve a registered scenario by name.
    pub fn get(name: &str) -> Option<Self> {
        scenario::get(name).map(|scenario| Self(Box::new(scenario)))
    }

    /// Resolve a scenario without converting unknown-name errors to absence.
    pub fn try_get(name: &str) -> Result<Option<Self>, String> {
        scenario::try_get(name).map(|scenario| scenario.map(|scenario| Self(Box::new(scenario))))
    }

    /// Whether `name` identifies a registered scenario.
    pub fn is_registered(name: &str) -> bool {
        scenario::is_registered(name)
    }


    #[cfg(feature = "js")]
    /// Create the external-loader prerequisite scenario.
    pub fn external_loader_fixture() -> Self {
        use api::interact::cheat;
        use host_play::external_loader::{BONES_COUNT, LIVE_SCENARIO, PREP_DEADLINE, PREREQ_SHOT};
        use scenario::{
            Proof, Scenario as AdapterScenario, ScenarioSettings, Seed, Step, StepKind, Wait,
        };

        Self(Box::new(AdapterScenario {
            name: LIVE_SCENARIO,
            seed: Seed {
                profiles: vec![("test", "test")],
                mainland: true,
            },
            steps: vec![
                Step {
                    name: "prepare twenty-five carried bones",
                    kind: StepKind::Perform {
                        send: Box::new(|client, _| {
                            cheat(client, scenario::tutorial::TUTORIAL_SETVAR);
                            cheat(client, scenario::tutorial::TUTORIAL_GETVAR);
                            cheat(client, "give bones 25");
                            true
                        }),
                    },
                    wait: Wait {
                        arm: Proof::Chat {
                            needle: scenario::tutorial::TUTORIAL_CHAT_NEEDLE,
                        },
                        budget_ticks: 200,
                    },
                },
                Step {
                    name: "relog so the inv tab binds",
                    kind: StepKind::Relog,
                    wait: Wait {
                        arm: Proof::SideTabAvailable { index: 3 },
                        budget_ticks: 600,
                    },
                },
                Step {
                    name: "reseed tutorial after relog and verify fresh value",
                    kind: StepKind::Repeat {
                        send: Box::new(|client, _| {
                            cheat(client, scenario::tutorial::TUTORIAL_SETVAR);
                            cheat(client, scenario::tutorial::TUTORIAL_GETVAR);
                            true
                        }),
                    },
                    wait: Wait {
                        arm: Proof::FreshTutorial,
                        budget_ticks: 200,
                    },
                },
            ],
            proof: Proof::Item {
                name: "Bones",
                count: BONES_COUNT,
            },
            companions: vec![],
            settings: ScenarioSettings {
                full_rate: true,
                require_mainland_base: true,
                deadline: PREP_DEADLINE,
                start_script: None,
                terminal_shot: Some(PREREQ_SHOT),
                // External-loader cells run at the engine's default tick and
                // request no world-wide `::speed` change.
                nav: scenario::ScenarioNav::default(),
                ..Default::default()
            },
        }))
    }

    /// Read the scalar scenario data needed by a front end.
    pub fn view(&self) -> ScenarioView<'_> {
        let scenario = &self.0;
        let settings = &scenario.settings;
        ScenarioView {
            name: scenario.name,
            profile_count: scenario.seed.profiles.len(),
            seed_profiles: &scenario.seed.profiles,
            mainland: scenario.seed.mainland,
            renderer: settings.renderer,
            only_render_selected: settings.only_render_selected,
            capture: settings.capture,
            full_rate: settings.full_rate,
            deadline: settings.deadline,
            terminal_shot: settings.terminal_shot,
            start_script: settings.start_script,
            start_file: settings.start_file,
            fleet_start: settings.fleet_start.map(|fleet| match fleet {
                scenario::FleetStart::ClueDuel => FleetStart::ClueDuel,
                scenario::FleetStart::JiveKq => FleetStart::JiveKq,
            }),
            wait_script_stop: settings.wait_script_stop,
            inject_companion_as: settings.inject_companion_as,
            companion_count: scenario.companions.len(),
            has_start_script_step: scenario
                .steps
                .iter()
                .any(|step| matches!(step.kind, scenario::StepKind::StartScript)),
            nav: ScenarioNav {
                allow_teleports: settings.nav.allow_teleports,
                allow_wilderness: settings.nav.allow_wilderness,
                show_nav_path: settings.nav.show_nav_path,
                hop_labels: settings.nav.hop_labels,
                hop_label_px: settings.nav.hop_label_px,
                collision_fill: settings.nav.collision_fill,
                nsew_labels: settings.nav.nsew_labels,
                client_trail: settings.nav.client_trail,
                component_flood: settings.nav.component_flood,
                camera_follow: settings.nav.camera_follow,
                engine_speed_ms: settings.nav.engine_speed_ms,
            },
        }
    }

    /// Copy fixture loadouts into API-owned values.
    pub fn fixture_loadouts(&self) -> Vec<api::loadout::Loadout> {
        self.0
            .settings
            .fixture_loadouts
            .unwrap_or(&[])
            .iter()
            .map(|row| {
                row.carry.iter().fold(
                    api::loadout::Loadout::new(row.name),
                    |loadout, &(item, quantity)| loadout.with_carry(item, quantity),
                )
            })
            .collect()
    }

    /// Build the scenario's final injected settings bag.
    pub fn settings_inject(&self) -> Option<Map<String, Value>> {
        scenario::settings_inject_map(self.0.settings.script_settings_inject)
    }

    /// Take dynamic scenario overrides so they can be merged once at Start.
    pub fn take_settings_overrides(&mut self) -> Option<Map<String, Value>> {
        self.0.settings.script_settings_overrides.take()
    }

    /// Scenario-owned card names required before a fleet Start.
    pub fn fleet_cards(&self, names: &[String]) -> Result<Vec<&'static str>, String> {
        self.0
            .settings
            .fleet_start
            .map(|fleet| {
                fleet
                    .members(names)
                    .map(|members| members.into_iter().map(|member| member.card).collect())
            })
            .unwrap_or_else(|| Ok(Vec::new()))
    }

    /// Copy the scenario's loadout and settings for fixture code.
    pub fn fixture_mode(self, mode: FixtureMode) -> Result<Self, String> {
        let adapter_mode = match mode {
            FixtureMode::Default => scenario::FixtureMode::Default,
            FixtureMode::Prepare => scenario::FixtureMode::Prepare,
            FixtureMode::RunPrepared => scenario::FixtureMode::RunPrepared,
        };
        scenario::apply_fixture_mode(*self.0, adapter_mode).map(|scenario| Self(Box::new(scenario)))
    }

    /// Whether the prepared scenario still contains setup cheats.
    pub fn run_prepared_has_setup_cheats(&self) -> bool {
        scenario::run_prepared_has_setup_cheats(&self.0)
    }

    /// Return the registered fixture preset, if one exists for this scenario.
    pub fn fixture_preset(&self) -> Result<&'static str, String> {
        scenario::fixture_preset_for(self.0.name)
    }

    /// Resolve the default identity receipt path.
    pub fn default_fixture_path(&self) -> PathBuf {
        scenario::default_fixture_path(self.0.name)
    }

    /// Resolve the default server save directory.
    pub fn default_fixture_sav_dir(&self) -> PathBuf {
        scenario::default_fixture_sav_dir(self.0.name)
    }

    /// Prepare a server-native fixture and return its durable identity.
    pub fn prepare_offline_fixture(
        &self,
        server_root: PathBuf,
        identity_path: PathBuf,
        sav_dir: PathBuf,
        usernames: Vec<String>,
        passwords: Vec<String>,
        vault_passphrase: String,
    ) -> Result<FixtureIdentity, String> {
        let fixture_preset = self.fixture_preset()?;
        let identity = scenario::prepare_offline_fixture(scenario::OfflinePrepareOpts {
            scenario: self.0.name.to_string(),
            fixture_preset: fixture_preset.to_string(),
            profile: "main".into(),
            server_root,
            identity_path,
            sav_dir,
            usernames,
            passwords,
            vault_passphrase,
            overwrite: true,
        })?;
        Ok(FixtureIdentity(Box::new(identity)))
    }
}

#[cfg(feature = "test-support")]
impl From<scenario::Scenario> for Scenario {
    fn from(scenario: scenario::Scenario) -> Self {
        Self(Box::new(scenario))
    }
}

/// Resolve the default durable fixture identity path for a scenario name.
pub fn default_fixture_path(name: &str) -> PathBuf {
    scenario::default_fixture_path(name)
}

impl ScenarioView<'_> {
    pub fn name(&self) -> &str {
        self.name
    }

    pub fn profile_count(&self) -> usize {
        self.profile_count
    }

    pub fn mainland(&self) -> bool {
        self.mainland
    }

    pub fn renderer(&self) -> bool {
        self.renderer
    }

    pub fn only_render_selected(&self) -> bool {
        self.only_render_selected
    }

    pub fn capture(&self) -> bool {
        self.capture
    }

    pub fn full_rate(&self) -> bool {
        self.full_rate
    }

    pub fn deadline(&self) -> Duration {
        self.deadline
    }

    pub fn terminal_shot(&self) -> Option<&str> {
        self.terminal_shot
    }

    pub fn start_script(&self) -> Option<&str> {
        self.start_script
    }

    pub fn start_file(&self) -> Option<&str> {
        self.start_file
    }

    pub fn fleet_start(&self) -> Option<FleetStart> {
        self.fleet_start
    }

    pub fn wait_script_stop(&self) -> Option<&str> {
        self.wait_script_stop
    }

    pub fn inject_companion_as(&self) -> Option<&str> {
        self.inject_companion_as
    }

    /// The seeded profiles' usernames, in seed order.
    pub fn seed_usernames(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.seed_profiles.iter().map(|(username, _)| *username)
    }

    /// How many companion slots the scenario drives beside its seed.
    pub fn companion_count(&self) -> usize {
        self.companion_count
    }

    /// Whether a step starts the scenario's script.
    pub fn has_start_script_step(&self) -> bool {
        self.has_start_script_step
    }

    pub fn nav(&self) -> ScenarioNav {
        self.nav
    }
}

impl ScenarioNav {
    pub fn allow_teleports(&self) -> bool {
        self.allow_teleports
    }
    pub fn allow_wilderness(&self) -> bool {
        self.allow_wilderness
    }
    pub fn show_nav_path(&self) -> bool {
        self.show_nav_path
    }
    pub fn hop_labels(&self) -> bool {
        self.hop_labels
    }
    pub fn hop_label_px(&self) -> i32 {
        self.hop_label_px
    }
    pub fn collision_fill(&self) -> bool {
        self.collision_fill
    }
    pub fn nsew_labels(&self) -> bool {
        self.nsew_labels
    }
    pub fn client_trail(&self) -> bool {
        self.client_trail
    }
    pub fn component_flood(&self) -> bool {
        self.component_flood
    }
    pub fn camera_follow(&self) -> bool {
        self.camera_follow
    }
    pub fn engine_speed_ms(&self) -> Option<u32> {
        self.engine_speed_ms
    }
}

impl FixtureIdentity {
    /// Read an identity receipt from disk.
    pub fn read_from(path: &Path) -> Result<Self, String> {
        scenario::FixtureIdentity::read_from(path).map(|identity| Self(Box::new(identity)))
    }

    /// Validate that this receipt belongs to the requested run.
    pub fn validate_for(&self, name: &str, profile_count: usize) -> Result<(), String> {
        self.0.validate_for(name, profile_count)
    }

    /// Install its saves into the selected engine players directory.
    pub fn install_into_players_dir(
        &self,
        path: &Path,
        overwrite: bool,
    ) -> Result<Vec<PathBuf>, String> {
        self.0.install_into_players_dir(path, overwrite)
    }

    pub fn scenario_name(&self) -> &str {
        &self.0.scenario
    }

    pub fn accounts(&self) -> Vec<(String, String, String, u64)> {
        self.0
            .accounts
            .iter()
            .map(|account| {
                (
                    account.username.clone(),
                    account.sav_path.clone(),
                    account.sav_sha256.clone(),
                    account.sav_bytes,
                )
            })
            .collect()
    }
}

impl LiveProfiles {
    /// Mint isolated credentials and a fresh passphrase for one live boot.
    pub fn mint(count: usize) -> Self {
        let names = mint_names(count);
        let entries = mint_entries(&names);
        let passphrase = vault_passphrase();
        Self {
            names,
            entries,
            passphrase,
        }
    }
    /// Prepare fixed null-raster test accounts for a headed boot.
    pub fn null_raster() -> Self {
        Self::from_entries(null_raster_entries())
    }

    /// Prepare stress-wall accounts for a headed boot.
    pub fn stress(count: usize) -> Self {
        Self::from_entries(stress_entries(count))
    }

    fn from_entries(entries: Vec<(String, String)>) -> Self {
        let names = entries.iter().map(|(name, _)| name.clone()).collect();
        Self {
            names,
            entries,
            passphrase: vault_passphrase(),
        }
    }

    /// Take the credentials and secret from an offline fixture identity.
    pub fn from_fixture_identity(identity: FixtureIdentity) -> Self {
        let identity = identity.0;
        Self {
            names: identity.usernames(),
            entries: identity.entries(),
            passphrase: identity.vault_passphrase.clone(),
        }
    }

    /// The isolated profile names in their prepared order.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// The profile credentials used to create the encrypted vault.
    pub fn entries(&self) -> &[(String, String)] {
        &self.entries
    }

    /// The passphrase for the encrypted vault.
    pub fn passphrase(&self) -> &str {
        &self.passphrase
    }

    /// Consume the profile data for offline fixture identity generation.
    pub fn into_fixture_parts(self) -> (Vec<String>, Vec<(String, String)>, String) {
        (self.names, self.entries, self.passphrase)
    }

    /// Create a panel live vault with the requested UID base and login mode.
    pub fn panel_vault(&self, uid_base: i32, auto_login: bool) -> PathBuf {
        panel_live_vault(&self.entries, uid_base, &self.passphrase, auto_login)
    }

    /// Create the TUI live vault.
    pub fn tui_vault(&self) -> PathBuf {
        tui_live_vault(&self.entries, &self.passphrase)
    }
}

/// Render a raw profile name in the screen-name form used by live scripts.
pub fn screen_name(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut capitalize = true;
    for character in input.chars() {
        if character == '_' {
            output.push(' ');
            capitalize = true;
        } else {
            output.push(if capitalize && character.is_ascii_lowercase() {
                character.to_ascii_uppercase()
            } else {
                character
            });
            capitalize = false;
        }
    }
    output
}

#[cfg(feature = "js")]
/// Resolve a registered checked-in live file fixture and its identity stem.
pub fn live_file_fixture(name: &str) -> Option<(PathBuf, String)> {
    Some((
        script_js::live_file_fixture_path(name)?,
        script_js::live_file_fixture_stem(name)?.to_string(),
    ))
}
/// Mint live account names without exposing the host-play helper crate.
fn mint_names(count: usize) -> Vec<String> {
    host_play::mint_live_names(count)
}

/// Mint passwords for the supplied isolated account names.
fn mint_entries(names: &[String]) -> Vec<(String, String)> {
    host_play::mint_live_entries(names)
}

/// Mint the passphrase for an ephemeral live vault.
fn vault_passphrase() -> String {
    host_play::live_vault_passphrase()
}

/// Return the normal live password for one isolated account name.
pub fn profile_password(name: &str) -> String {
    host_play::profile_password(name)
}

/// Create the panel's null-raster account entries.
pub fn null_raster_entries() -> Vec<(String, String)> {
    ["test", "test2"]
        .into_iter()
        .map(|name| (name.to_string(), profile_password(name)))
        .collect()
}

/// Create the panel's stress-wall account entries.
pub fn stress_entries(count: usize) -> Vec<(String, String)> {
    (0..count.max(1))
        .map(|index| {
            let name = format!("s{index:02}");
            (name.clone(), profile_password(&name))
        })
        .collect()
}

/// Create an encrypted panel live vault with a caller-selected UID base.
pub fn panel_live_vault(
    entries: &[(String, String)],
    uid_base: i32,
    passphrase: &str,
    auto_login: bool,
) -> PathBuf {
    static SERIAL: AtomicUsize = AtomicUsize::new(0);
    create_live_vault(
        entries,
        uid_base,
        passphrase,
        auto_login,
        "274bot-panel-live",
        &SERIAL,
    )
}

/// Create the TUI's encrypted live vault.
fn tui_live_vault(entries: &[(String, String)], passphrase: &str) -> PathBuf {
    static SERIAL: AtomicUsize = AtomicUsize::new(0);
    create_live_vault(
        entries,
        274_000_001,
        passphrase,
        true,
        "274bot-tui-live",
        &SERIAL,
    )
}

fn create_live_vault(
    entries: &[(String, String)],
    uid_base: i32,
    passphrase: &str,
    auto_login: bool,
    prefix: &str,
    serial: &AtomicUsize,
) -> PathBuf {
    use vault::{Profile, ProfileSettings, Vault};

    let serial = serial.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(if prefix == "274bot-tui-live" {
        format!("{prefix}-{}-{}-{serial}", std::process::id(), entries.len())
    } else {
        format!(
            "{prefix}-{}-{}-{uid_base}-{serial}",
            std::process::id(),
            entries.len()
        )
    });
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("vault");
    if path.exists() {
        std::fs::remove_file(&path).unwrap();
    }
    let mut vault = Vault::create(&path, passphrase).unwrap();
    for (index, (username, password)) in entries.iter().enumerate() {
        vault
            .upsert(Profile {
                username: username.clone(),
                password: password.clone().into(),
                uid: uid_base + index as i32,
                settings: ProfileSettings {
                    auto_login,
                    ..ProfileSettings::default()
                },
            })
            .unwrap();
    }
    path
}

/// Parse the optional live soak budget.
pub fn budget_s_from_env() -> Option<Duration> {
    scenario::budget_s_from_env()
}

/// Format the stable UTC shot-label timestamp.
pub fn shot_stamp_utc(now: std::time::SystemTime) -> String {
    scenario::shot::stamp_utc(now)
}

/// Create a unique shot directory for one headed live run.
pub fn create_shot_run_dir() -> Result<PathBuf, String> {
    scenario::shot::create_run_dir().map_err(|error| error.to_string())
}

/// Write a captured headed shot and its scenario JSON sidecar.
pub fn write_shot(
    directory: &Path,
    label: &str,
    rgba: &[u8],
    width: u32,
    height: u32,
    snapshot_json: &str,
) -> std::io::Result<PathBuf> {
    scenario::shot::write_shot(directory, label, rgba, width, height, snapshot_json)
}

#[cfg(feature = "js")]
/// Resolve the scenario's shared external-loader name.
pub fn external_loader_scenario_name() -> &'static str {
    host_play::external_loader::LIVE_SCENARIO
}

#[cfg(feature = "js")]
/// Resolve the external-loader command-line scenario token.
pub fn external_loader_live_name() -> &'static str {
    host_play::external_loader::LIVE_NAME
}

#[cfg(feature = "js")]
/// Stable terminal screenshot label for the external-loader proof.
pub fn external_loader_terminal_shot() -> &'static str {
    host_play::external_loader::TERMINAL_SHOT
}

#[cfg(feature = "js")]
/// Stable prereq screenshot label for the external-loader proof.
pub fn external_loader_prereq_shot() -> &'static str {
    host_play::external_loader::PREREQ_SHOT
}

#[cfg(feature = "js")]
/// Read the remaining external-loader names and limits used by the panel.
pub fn external_loader_script_name() -> &'static str {
    host_play::external_loader::SCRIPT_NAME
}

#[cfg(feature = "js")]
/// External loader's successful unchanged-reload result text.
pub fn external_loader_nothing_changed() -> &'static str {
    host_play::external_loader::NOTHING_CHANGED
}

#[cfg(feature = "js")]
/// External loader's stable frozen source digest.
pub fn external_loader_frozen_sha256() -> &'static str {
    host_play::external_loader::FROZEN_SHA256
}

#[cfg(feature = "js")]
/// External loader's bones item id.
pub fn external_loader_bones_id() -> i32 {
    host_play::external_loader::BONES_ID
}

#[cfg(feature = "js")]
/// External loader's prayer stat id.
pub fn external_loader_prayer_stat_id() -> i32 {
    host_play::external_loader::PRAYER_STAT_ID
}

#[cfg(feature = "js")]
/// External loader's required carried-bones count.
pub fn external_loader_bones_count() -> i32 {
    host_play::external_loader::BONES_COUNT
}

#[cfg(feature = "js")]
/// Resolve the external-loader source used by the proof.
pub fn resolve_external_loader_source(path: Option<&Path>) -> Result<PathBuf, String> {
    host_play::external_loader::resolve_source(path)
}

#[cfg(feature = "js")]
/// Freeze the external-loader source into an owned run-local path.
pub fn materialize_external_loader_source(path: &Path) -> Result<PathBuf, String> {
    host_play::external_loader::materialize_owned_source(path)
}

/// Count active TCP sockets to the selected live endpoint.
pub fn count_tcp_to(host: &str, port: u16) -> Option<usize> {
    host_play::count_tcp_to(host, port)
}

/// Count the process RSS used by the live stress proof.
pub fn process_rss_kib() -> u64 {
    host_play::sample_process().0
}

impl<Io> crate::OperatorSession<Io> {
    /// Validate and retain the selected live core-witness mode.
    pub fn resolve_live_core(
        &self,
        scenario_name: &str,
        profile_count: usize,
        catalog_enabled: bool,
        pair_enabled: bool,
    ) -> Result<LiveCoreMode, String> {
        self.live_runner()
            .resolve_core(scenario_name, profile_count, catalog_enabled, pair_enabled)
    }

    /// Configure the retained witness mode against the current Play.
    pub fn configure_live_core(&self, names: &[String]) -> Result<(), String> {
        let play = self
            .play()
            .ok_or_else(|| "live core watch needs the unlocked play".to_string())?;
        self.live_runner().configure_core(play, names)
    }

    /// Build a scenario runner with the current Play's world and diagnostics.
    pub fn live_scenario_runner(
        &self,
        scenario: Scenario,
        names: &[String],
        host: &str,
        port: u16,
        budget: Option<Duration>,
    ) -> Runner {
        let world = self.play().and_then(|play| play.world());
        let mut runner = Runner::with_world(scenario, world);
        runner.configure_for_play(self, host, port);
        runner.set_live_names(names);
        if let Some(budget) = budget {
            runner.set_deadline(budget);
        }
        runner
    }
    #[cfg(feature = "js")]
    /// Resolve, freeze, and arm the external-loader source for one account.
    pub fn configure_live_external_loader(
        &self,
        account: String,
        source: Option<&Path>,
    ) -> Result<(), String> {
        let source = resolve_external_loader_source(source)?;
        let owned = materialize_external_loader_source(&source)?;
        self.live_runner().configure_external_loader(account, owned);
        Ok(())
    }

    #[cfg(feature = "js")]
    /// Pump one pending external-loader proof operation.
    pub fn pump_live_external_loader(
        &mut self,
        scripts: &mut crate::Scripts,
        selection: &mut Option<crate::scripts::ScriptChoice>,
        statuses: &crate::StatusBuf,
        now: Instant,
    ) {
        let runner = self.live_runner().share();
        runner.pump_external_loader(scripts, self, selection, statuses, now);
    }

    /// Prepare the headed stress-wall profiles before their slots spawn.
    pub fn prepare_live_stress_profiles(&mut self, entries: &[(String, String)], full_rate: bool) {
        for (index, (name, _)) in entries.iter().enumerate() {
            if let Some(mut profile) = self.vault().and_then(|vault| vault.get(name)).cloned() {
                profile.settings.lowmem = true;
                profile.settings.raster = if full_rate || index == 0 {
                    vault::RasterMode::Gpu
                } else {
                    vault::RasterMode::Off
                };
                let _ = self.save_profile(profile, crate::ArmMirror::none(), "stress");
            }
        }
    }

    /// Observe the scenario-named clean-stop receipt for one profile.
    pub fn live_script_self_stop_observed(&self, profile: &str, needle: &str) -> bool {
        let Some(play) = self.play() else {
            return false;
        };
        host_play::live_gate::script_self_stop_observed(
            play.script_state(profile),
            play.script_lifecycle_receipt(profile).as_ref(),
            needle,
        )
    }

    /// Copy lifecycle receipt details for the TUI's optional failure trace.
    pub fn live_script_lifecycle_debug(&self, profile: &str) -> Option<String> {
        let receipt = self.play()?.script_lifecycle_receipt(profile)?;
        Some(format!(
            "generation={} state={:?} tick={} reason={:?}",
            receipt.runtime_generation, receipt.state, receipt.tick, receipt.reason
        ))
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn screen_name_matches_the_client_conversion() {
        for raw in ["", "live_bot_7", "_a__b", "CAPS", "a_9_é", "😄_x"] {
            assert_eq!(
                super::screen_name(raw),
                client::util::JString::to_screen_name(raw),
                "{raw:?}",
            );
        }
    }
    #[test]
    fn quester_skip_fixture_runs_the_two_quest_queue() {
        let scenario = super::Scenario::get("quester_skip").expect("skip scenario");
        assert!(super::Scenario::try_get("quester_skip")
            .expect("scenario lookup")
            .is_some());
        let settings = scenario.settings_inject().expect("queue settings");
        assert_eq!(
            settings.get("quests"),
            Some(&serde_json::json!(["cook", "sheep"]))
        );
        assert_eq!(
            settings.get("order_override"),
            Some(&serde_json::json!(["cook", "sheep"]))
        );
        let start = scenario
            .0
            .steps
            .iter()
            .position(|step| matches!(&step.kind, scenario::StepKind::StartScript))
            .expect("compiled start");
        assert_eq!(scenario.0.steps.len(), start + 2);
        assert!(matches!(
            &scenario.0.steps[start + 1].wait.arm,
            scenario::Proof::QuestDone { name } if *name == "Sheep Shearer"
        ));
        assert!(matches!(
            &scenario.0.proof,
            scenario::Proof::QuestDone { name } if *name == "Sheep Shearer"
        ));
        assert!(scenario.view().terminal_shot().is_some());
    }
    #[test]
    fn pair_script_elapsed_starts_only_after_both_catalog_slots_start() {
        let started = std::time::Instant::now();
        let mut timing = Some(host_play::live_gate::LiveTiming::new(
            started,
            Some(started + std::time::Duration::from_secs(180)),
        ));
        let mut pending = vec![
            host_play::live_start::PendingCatalogStart::compiled(
                "runner",
                crate::scripts::CompiledCardId::QUESTER.to_script(),
                serde_json::Map::new(),
            ),
            host_play::live_start::PendingCatalogStart::compiled(
                "spinner",
                crate::scripts::CompiledCardId::QUESTER.to_script(),
                serde_json::Map::new(),
            ),
        ];
        let accepted = host_play::live_start::StartScriptPump::Continue;
        super::mark_live_script_start(&mut timing, &pending, true, &accepted, started);
        assert!(!timing.as_ref().unwrap().script_started());

        pending[0].started = true;
        super::mark_live_script_start(
            &mut timing,
            &pending,
            true,
            &accepted,
            started + std::time::Duration::from_secs(4),
        );
        assert!(!timing.as_ref().unwrap().script_started());

        pending[1].started = true;
        let script_started = started + std::time::Duration::from_secs(7);
        super::mark_live_script_start(&mut timing, &pending, true, &accepted, script_started);
        let timing = timing.unwrap();
        assert_eq!(
            timing.setup_elapsed(script_started + std::time::Duration::from_secs(1)),
            std::time::Duration::from_secs(7)
        );
        assert_eq!(
            timing.script_elapsed(script_started + std::time::Duration::from_secs(1)),
            Some(std::time::Duration::from_secs(1))
        );
    }
}
