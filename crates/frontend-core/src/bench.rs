//! The opt-in memory benchmark (`memory-profile`, `BOT_MEMORY_N`): its
//! configuration, the benchmark run over a play session, the startup marks
//! and the per-frame timer the panel records.

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::launch::ProfileClass;
use crate::OperatorSession;

/// Which slots draw during a benchmark run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderPolicy {
    /// Members may draw; focus rotates every 30 s.
    RotatingAll,
    /// Fixed focus 0, only the selected slot draws.
    FixedOne,
    /// Fixed focus 0 at full rate; the others simulate only.
    FocusedOne,
    /// Fixed focus 0 at full rate; the others paint at 1 fps.
    FocusedPlusBackground,
    /// The panel `stress50` wall: one drawing head, 49 simulation-only slots.
    Stress50,
    /// The panel `stress50_full` wall: every member draws at full rate.
    Stress50Full,
}

impl RenderPolicy {
    fn from_host(policy: host_play::memory::RenderPolicy) -> Self {
        use host_play::memory::RenderPolicy as P;
        match policy {
            P::RotatingAll => Self::RotatingAll,
            P::FixedOne => Self::FixedOne,
            P::FocusedOne => Self::FocusedOne,
            P::FocusedPlusBackground => Self::FocusedPlusBackground,
            P::Stress50 => Self::Stress50,
            P::Stress50Full => Self::Stress50Full,
        }
    }
}

/// The benchmark configuration read from the environment.
#[derive(Debug)]
pub struct BenchConfig(Box<host_play::memory::Config>);

impl BenchConfig {
    /// The configuration `BOT_MEMORY_*` asks for, if the benchmark is on.
    pub fn from_env() -> Result<Option<Self>, BenchError> {
        host_play::memory::Config::from_env()
            .map(|config| config.map(|c| Self(Box::new(c))))
            .map_err(|error| BenchError(BenchStep::Config(error)))
    }
}

/// Why a benchmark step failed: the sentence `host-play` reports for that
/// step. Callers show it; the step stays private.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error(transparent)]
pub struct BenchError(BenchStep);

/// The benchmark step that failed, with `host-play`'s sentence.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
enum BenchStep {
    /// The `BOT_MEMORY_*` environment is invalid.
    #[error("{0}")]
    Config(String),
    /// The profile is not a live local one.
    #[error("{0}")]
    LiveRequired(String),
    /// Preparing the run's vault and names failed.
    #[error("{0}")]
    Prepare(String),
    /// Seeding the run's navigation failed.
    #[error("{0}")]
    SeedNav(String),
    /// The observation run failed (already recorded per slot).
    #[error("{0}")]
    Run(String),
}

/// Refuse a benchmark run against a profile that is not a live local one.
pub fn require_live_benchmark(class: ProfileClass) -> Result<(), BenchError> {
    host_play::memory::require_live_benchmark(class.to_host())
        .map_err(|error| BenchError(BenchStep::LiveRequired(error)))
}

/// One benchmark run: its temporary vault, minted names and render policy.
pub struct BenchRun(Box<host_play::memory::Run>);

impl BenchRun {
    /// Prepare the run's vault and names for `frontend` (`"panel"`, `"tui"`).
    pub fn prepare(config: BenchConfig, frontend: &'static str) -> Result<Self, BenchError> {
        host_play::memory::Run::prepare_unseeded(*config.0, frontend)
            .map(|run| Self(Box::new(run)))
            .map_err(|error| BenchError(BenchStep::Prepare(error)))
    }

    /// The run's minted profile names.
    pub fn names(&self) -> &[String] {
        &self.0.names
    }

    /// The run's temporary vault.
    pub fn vault(&self) -> &Path {
        &self.0.vault
    }

    /// The run's vault passphrase.
    pub fn passphrase(&self) -> &str {
        &self.0.pass
    }

    /// Which slots draw.
    pub fn render_policy(&self) -> RenderPolicy {
        RenderPolicy::from_host(self.0.render_policy)
    }

    /// The index of the name to focus now.
    pub fn focus_index(&self) -> usize {
        self.0.focus_index()
    }

    /// Seed the run's navigation from `core`'s play session.
    pub fn bind_seed_nav<Io>(&self, core: &OperatorSession<Io>) -> Result<(), BenchError> {
        self.0
            .bind_seed_nav(host_play::memory::SeedNav::FromPlay {
                world: core.nav_world(),
                obj_names: core.obj_names(),
                map_members: core.map_members(),
            })
            .map_err(|error| BenchError(BenchStep::SeedNav(error)))
    }

    /// Advance the run against `core`'s play session: `Ok(true)` once the
    /// observation completed; without a play session nothing happens.
    pub fn poll<Io>(&mut self, core: &OperatorSession<Io>) -> Result<bool, BenchError> {
        match core.play() {
            Some(play) => self
                .0
                .poll(play)
                .map_err(|error| BenchError(BenchStep::Run(error))),
            None => Ok(false),
        }
    }

    /// Put a test run at the terminal observation boundary without a live server.
    #[cfg(any(test, feature = "test-support"))]
    pub fn finish_for_test(&mut self) {
        self.0.finish_for_test();
    }
}

/// A startup milestone the benchmark record times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupMark {
    /// The OS window exists.
    WindowCreated,
    /// Adapter, device, surface and the ImGui renderer are ready.
    GpuReady,
}

/// Record `mark` (first time only).
pub fn mark_startup(mark: StartupMark) {
    host_play::memory::mark_startup(match mark {
        StartupMark::WindowCreated => host_play::memory::StartupMark::WindowCreated,
        StartupMark::GpuReady => host_play::memory::StartupMark::GpuReady,
    });
}

/// The graphics adapter identity the run record names.
#[derive(Debug, Default)]
pub struct AdapterIdentity {
    name: String,
    backend: String,
    device_type: String,
    driver: String,
    driver_info: String,
    vendor: u32,
    device: u32,
    pci_bus_id: String,
}

impl AdapterIdentity {
    /// The adapter's name, backend and device type, as reported.
    pub fn new(name: String, backend: String, device_type: String) -> Self {
        Self {
            name,
            backend,
            device_type,
            ..Self::default()
        }
    }

    /// The driver name and driver details.
    pub fn with_driver(mut self, driver: String, driver_info: String) -> Self {
        self.driver = driver;
        self.driver_info = driver_info;
        self
    }

    /// The PCI vendor and device ids and bus id.
    pub fn with_pci(mut self, vendor: u32, device: u32, pci_bus_id: String) -> Self {
        self.vendor = vendor;
        self.device = device;
        self.pci_bus_id = pci_bus_id;
        self
    }
}

/// Record the adapter identity once for the run record (writes no log
/// line).
pub fn record_adapter(adapter: AdapterIdentity) {
    host_play::memory::record_adapter(host_play::memory::AdapterRecord {
        name: adapter.name,
        backend: adapter.backend,
        device_type: adapter.device_type,
        driver: adapter.driver,
        driver_info: adapter.driver_info,
        vendor: adapter.vendor,
        device: adapter.device,
        pci_bus_id: adapter.pci_bus_id,
    });
}

/// Times one panel UI frame for the run record while it lives. The host
/// timer stays in a process slot (no allocation per frame); the panel times
/// one frame at a time on its UI thread.
pub struct PanelFrameTimer {
    _started: (),
}

static PANEL_FRAME: Mutex<Option<host_play::memory::PanelFrameTimer>> = Mutex::new(None);

fn panel_frame() -> MutexGuard<'static, Option<host_play::memory::PanelFrameTimer>> {
    PANEL_FRAME.lock().unwrap_or_else(PoisonError::into_inner)
}

impl PanelFrameTimer {
    /// Start timing a frame.
    pub fn start() -> Self {
        let timer = host_play::memory::PanelFrameTimer::start();
        *panel_frame() = Some(timer);
        Self { _started: () }
    }

    /// The frame's swapchain image was just presented (call right after
    /// `present()`).
    pub fn presented(&self) {
        if let Some(timer) = panel_frame().as_ref() {
            timer.presented();
        }
    }
}

impl Drop for PanelFrameTimer {
    fn drop(&mut self) {
        // Dropping the host timer records the frame time.
        let timer = panel_frame().take();
        drop(timer);
    }
}
