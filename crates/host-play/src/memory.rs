//! Opt-in frontend memory benchmark harness.
//!
//! No effect unless `BOT_MEMORY_N` is set. Counting allocator is installed
//! only by frontend binaries — no logging or allocation inside allocator
//! callbacks. Sample fields are separate domains; never sum them and never
//! equate allocation counts with RSS.

use crate::Play;
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
static LIVE_BYTES: AtomicU64 = AtomicU64::new(0);

/// Installed only by benchmark-enabled frontend binaries; no logging or
/// allocation inside allocator callbacks. Requested Rust bytes exclude
/// V8 / native / GPU allocators.
pub struct CountingAllocator;

#[cfg(not(feature = "memory-profile-no-alloc"))]
pub type BenchmarkAllocator = CountingAllocator;
#[cfg(not(feature = "memory-profile-no-alloc"))]
pub const BENCHMARK_ALLOCATOR: BenchmarkAllocator = CountingAllocator;
#[cfg(feature = "memory-profile-no-alloc")]
pub type BenchmarkAllocator = System;
#[cfg(feature = "memory-profile-no-alloc")]
pub const BENCHMARK_ALLOCATOR: BenchmarkAllocator = System;

/// Process CPU time, distinct from overlapping per-thread wall durations.
fn process_cpu_seconds() -> Option<(f64, f64)> {
    #[cfg(unix)]
    {
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } != 0 {
            return None;
        }
        let seconds = |v: libc::timeval| v.tv_sec as f64 + v.tv_usec as f64 / 1_000_000.0;
        Some((seconds(usage.ru_utime), seconds(usage.ru_stime)))
    }
    #[cfg(not(unix))]
    None
}


unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            ALLOCS.fetch_add(1, Relaxed);
            ALLOC_BYTES.fetch_add(layout.size() as u64, Relaxed);
            LIVE_BYTES.fetch_add(layout.size() as u64, Relaxed);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(layout);
        if !p.is_null() {
            ALLOCS.fetch_add(1, Relaxed);
            ALLOC_BYTES.fetch_add(layout.size() as u64, Relaxed);
            LIVE_BYTES.fetch_add(layout.size() as u64, Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        System.dealloc(p, layout);
        LIVE_BYTES.fetch_sub(layout.size() as u64, Relaxed);
    }

    unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let new = System.realloc(p, layout, size);
        if !new.is_null() {
            ALLOCS.fetch_add(1, Relaxed);
            ALLOC_BYTES.fetch_add(size as u64, Relaxed);
            LIVE_BYTES.fetch_add(size as u64, Relaxed);
            LIVE_BYTES.fetch_sub(layout.size() as u64, Relaxed);
        }
        new
    }
}

/// Snapshot of the counting allocator atomics.
///
/// Meaningful only after a frontend installs [`CountingAllocator`] as the
/// global allocator. Until then, prefer JSON null over these zeros.
pub fn rust_allocator_counts() -> (u64, u64, u64) {
    (
        ALLOCS.load(Relaxed),
        ALLOC_BYTES.load(Relaxed),
        LIVE_BYTES.load(Relaxed),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Workload {
    Idle,
    SeededIdle,
    Active,
    Lifecycle,
}

impl Workload {
    pub fn as_str(self) -> &'static str {
        match self {
            Workload::Idle => "idle",
            Workload::SeededIdle => "seeded-idle",
            Workload::Active => "active",
            Workload::Lifecycle => "lifecycle",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub n: usize,
    pub workload: Workload,
    pub warmup: Duration,
    pub observe: Duration,
}

impl Config {
    /// Parse opt-in env. `Ok(None)` when `BOT_MEMORY_N` is unset (normal play).
    pub fn from_env() -> Result<Option<Self>, String> {
        let Ok(n) = std::env::var("BOT_MEMORY_N") else {
            return Ok(None);
        };
        let n = parse_n(&n)?;
        let workload = match std::env::var("BOT_MEMORY_WORKLOAD").as_deref() {
            Err(_) | Ok("idle") => Workload::Idle,
            Ok("seeded-idle") => Workload::SeededIdle,
            Ok("active") => Workload::Active,
            Ok("lifecycle") => Workload::Lifecycle,
            _ => return Err("BOT_MEMORY_WORKLOAD must be idle, seeded-idle, active, or lifecycle".into()),
        };
        let duration = |key: &str, default: u64| -> Result<Duration, String> {
            let secs = match std::env::var(key) {
                Ok(s) => s.parse::<u64>().map_err(|_| format!("invalid {key}"))?,
                Err(_) => default,
            };
            if secs == 0 {
                return Err(format!("{key} must be positive"));
            }
            Ok(Duration::from_secs(secs))
        };
        Ok(Some(Self {
            n,
            workload,
            warmup: duration("BOT_MEMORY_WARMUP_S", 120)?,
            observe: duration("BOT_MEMORY_OBSERVE_S", 600)?,
        }))
    }
}

pub fn parse_n(n: &str) -> Result<usize, String> {
    match n {
        "1" => Ok(1),
        "16" => Ok(16),
        "32" => Ok(32),
        "128" => Ok(128),
        _ => Err("BOT_MEMORY_N must be 1, 16, 32, or 128".into()),
    }
}

/// Requested panel render cell. Logged as requested metadata, not observed GPU proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderPolicy {
    /// Default panel benchmark: members may draw; focus rotates every 30s.
    RotatingAll,
    /// Historical `BOT_MEMORY_SINGLE_RENDERER`: fixed focus 0, only selected draws.
    FixedOne,
    /// Explicit low-end: fixed focus 0 full-rate GPU; others simulation-only.
    FocusedOne,
    /// Explicit low-end: fixed focus 0 full-rate; others 1 fps skip-paint.
    FocusedPlusBackground,
}

impl RenderPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RotatingAll => "rotating-all",
            Self::FixedOne => "fixed-one",
            Self::FocusedOne => "focused-one",
            Self::FocusedPlusBackground => "focused-plus-background",
        }
    }

    /// Fixed slot-zero focus for single-seat and background-renderer cells.
    pub fn pins_focus(self) -> bool {
        !matches!(self, Self::RotatingAll)
    }
}

/// Parse requested panel policy from env. Panel-only flags on TUI error before run.
pub fn parse_render_policy(frontend: &str) -> Result<RenderPolicy, String> {
    let single = std::env::var("BOT_MEMORY_SINGLE_RENDERER").as_deref() == Ok("1");
    let policy = std::env::var("BOT_MEMORY_RENDER_POLICY").ok();
    let policy = policy
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if frontend == "tui" && (single || policy.is_some()) {
        return Err(
            "BOT_MEMORY_SINGLE_RENDERER / BOT_MEMORY_RENDER_POLICY require panel frontend".into(),
        );
    }
    match (single, policy) {
        (false, None) => Ok(RenderPolicy::RotatingAll),
        (true, None) => Ok(RenderPolicy::FixedOne),
        (false, Some("fixed-one")) => Ok(RenderPolicy::FixedOne),
        (false, Some("focused-one")) => Ok(RenderPolicy::FocusedOne),
        (false, Some("focused-plus-background")) => Ok(RenderPolicy::FocusedPlusBackground),
        (false, Some("rotating-all")) => Ok(RenderPolicy::RotatingAll),
        (true, Some(_)) => Err(
            "BOT_MEMORY_SINGLE_RENDERER conflicts with BOT_MEMORY_RENDER_POLICY".into(),
        ),
        (false, Some(other)) => Err(format!(
            "BOT_MEMORY_RENDER_POLICY must be rotating-all, fixed-one, focused-one, or focused-plus-background; got {other}"
        )),
    }
}

/// One jsonl sample line. Metric fields are siblings — never summed.
///
/// Missing components are JSON `null` (not zero pretending to be a measurement).
#[derive(Clone, Debug)]
pub struct Sample {
    pub frontend: String,
    pub n: usize,
    pub workload: Workload,
    pub elapsed_s: f64,
    pub phase: String,
    pub ready: usize,
    pub active: usize,
    pub resident_bytes: Option<u64>,
    pub peak_resident_bytes: Option<u64>,
    pub rust_allocations: Option<u64>,
    pub rust_allocated_bytes: Option<u64>,
    pub rust_live_bytes: Option<u64>,
    pub snapshot_inflight_bytes: Option<u64>,
    pub snapshot_inflight_capacity: Option<u64>,
    pub v8_used_bytes: Option<u64>,
    pub v8_total_bytes: Option<u64>,
    pub gpu_tracked_bytes: Option<u64>,
}

impl Sample {
    /// Serialize as a JSON object with sibling metric fields (null when missing).
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "frontend": self.frontend,
            "n": self.n,
            "workload": self.workload.as_str(),
            "elapsed_s": self.elapsed_s,
            "phase": self.phase,
            "ready": self.ready,
            "active": self.active,
            "resident_bytes": self.resident_bytes,
            "peak_resident_bytes": self.peak_resident_bytes,
            "rust_allocations": self.rust_allocations,
            "rust_allocated_bytes": self.rust_allocated_bytes,
            "rust_live_bytes": self.rust_live_bytes,
            "snapshot_inflight_bytes": self.snapshot_inflight_bytes,
            "snapshot_inflight_capacity": self.snapshot_inflight_capacity,
            "v8_used_bytes": self.v8_used_bytes,
            "v8_total_bytes": self.v8_total_bytes,
            "gpu_tracked_bytes": self.gpu_tracked_bytes,
        })
    }
}

/// Live bench gate: `LIVE=1` and the local engine target. Frontends call
/// this when `Config::from_env` returns `Some` — unit `Config` tests stay
/// free of the live requirement.
pub fn require_live_benchmark() -> Result<(), String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Err("memory benchmark requires LIVE=1 and the local target".into());
    }
    if client::bot_target() != client::BotTarget::Local {
        return Err("memory benchmark requires LIVE=1 and the local target".into());
    }
    Ok(())
}

struct Seed {
    runner: scenario::ScenarioRunner,
    started: bool,
}

/// Per-slot Thiever seed runners installed by [`Run::prepare`] for
/// seeded-idle/active/lifecycle. Unseeded idle leaves this empty.
static SEEDS: Mutex<Option<HashMap<String, Arc<Mutex<Seed>>>>> = Mutex::new(None);

/// Called from the existing frontend slot observe hook. Drives the Thiever
/// scenario seed/proof; unseeded idle installs no seeds.
pub(crate) fn client_frame(c: &mut client::client::Client, name: &str, hold: bool) {
    crate::memory_diagnostics::frame(c, name, hold);
    let seed = {
        let seeds = SEEDS.lock().unwrap();
        seeds.as_ref().and_then(|m| m.get(name).cloned())
    };
    let Some(seed) = seed else {
        return;
    };
    let mut seed = seed.lock().unwrap();
    if seed.runner.on_start_script() && !seed.started {
        return;
    }
    seed.runner.tick_with_hold(c, hold);
}

/// The same sustained Thiever setup, ending before any script is loaded.
/// Keep the original idle workload available as the unseeded login control.
fn seeded_idle_scenario() -> scenario::Scenario {
    let mut scenario = scenario::thiever_sustained_scenario();
    let start = scenario.steps.iter().position(|step| matches!(step.kind, scenario::StepKind::StartScript)).expect("Thiever StartScript");
    let arrival = scenario.steps.iter().find(|step| step.name == "seed stats, food, and tele to the guard stand").expect("Thiever seed").wait.arm;
    scenario.steps.truncate(start);
    scenario.proof = arrival;
    scenario.settings.start_script = None;
    scenario.settings.terminal_shot = None;
    scenario
}

fn seed_runner(scenario: scenario::Scenario, name: &str, world: Option<Arc<nav::world::NavWorld>>) -> Seed {
    let mut runner = scenario::ScenarioRunner::with_world(scenario, world);
    runner.set_live_names(&[name.to_owned()]);
    runner.set_deadline(Duration::from_secs(1800));
    Seed { runner, started: false }
}

type ScriptCard = (
    script::JsCard,
    serde_json::Map<String, serde_json::Value>,
    Vec<(String, String)>,
);

/// Prepared memory-benchmark run shared by panel-play and tui-play.
pub struct Run {
    pub config: Config,
    pub names: Vec<String>,
    pub vault: PathBuf,
    pub pass: String,
    frontend: &'static str,
    started: Instant,
    warm: Option<Instant>,
    observing: Option<Instant>,
    last_sample: Option<Instant>,
    lifecycle_cycle: u64,
    stopped: bool,
    teardown: Option<Instant>,
    card: Option<ScriptCard>,
    output: std::fs::File,
    diagnostics: bool,
    /// `BOT_MEMORY_FAILURE_CAPTURE=1`: one failure-boundary qualification row +
    /// isolate stop-reason cache, without the periodic diagnostic sidecar.
    pub failure_capture: bool,
    /// Latched once a failure-boundary write is attempted (success or I/O fail).
    failure_boundary_attempted: bool,
    /// Historical `BOT_MEMORY_SINGLE_RENDERER=1` only (old metadata summaries).
    pub single_renderer: bool,
    /// Requested panel draw/focus cell; TUI leaves this at [`RenderPolicy::RotatingAll`].
    pub render_policy: RenderPolicy,
    diagnostic_output: Option<std::fs::File>,
    qualification_output: std::fs::File,
}

/// Where non-idle seed runners get their [`nav::world::NavWorld`].
///
/// Frontends pass [`SeedNav::FromPlay`] with `Play::world()` so the pack is
/// decoded once. Unit tests without a Play use [`SeedNav::LoadDefault`].
#[derive(Clone)]
pub enum SeedNav {
    /// Decode the default pack once for this prepare (no Play yet).
    LoadDefault,
    /// Reuse the Play-owned world as-is — `None` preserves missing-pack
    /// behavior (no second decode attempt).
    FromPlay(Option<Arc<nav::world::NavWorld>>),
}

impl Run {
    /// Mint ephemeral accounts, throwaway vault, optional Thiever card, and
    /// install seed runners via [`SeedNav::LoadDefault`]. Prefer
    /// [`prepare_with_seed_nav`] from panel/TUI so seeds share `Play::world`.
    /// Idle modes: no card or RS2B0T. SeededIdle runs the Thiever setup only.
    /// Active/lifecycle: RS2B0T required.
    pub fn prepare(config: Config, frontend: &'static str) -> Result<Self, String> {
        Self::prepare_with_seed_nav(config, frontend, SeedNav::LoadDefault)
    }

    /// Like [`prepare`], but seed runners take navigation from `seed_nav`
    /// instead of always decoding a second pack copy.
    pub fn prepare_with_seed_nav(
        config: Config,
        frontend: &'static str,
        seed_nav: SeedNav,
    ) -> Result<Self, String> {
        let run = Self::prepare_unseeded(config, frontend)?;
        run.bind_seed_nav(seed_nav)?;
        Ok(run)
    }

    /// Vault, names, card, and sample outputs — **no** seed runners yet.
    /// Frontends start `Play` next, then [`bind_seed_nav`] with
    /// [`SeedNav::FromPlay`](`play.world()`) before spawning slots.
    pub fn prepare_unseeded(config: Config, frontend: &'static str) -> Result<Self, String> {
        // Fail closed on panel-only / conflicting flags before minting vaults.
        let render_policy = parse_render_policy(frontend)?;
        let single_renderer =
            std::env::var("BOT_MEMORY_SINGLE_RENDERER").as_deref() == Ok("1");
        client::profiling::enable();
        if std::env::var("BOT_SCHEDULING_PROFILE").as_deref() == Ok("1") { host::cadence::enable(); }
        if std::env::var("BOT_RENDER_PROFILE").as_deref() == Ok("1") {
            host::render_profile::enable();
            if std::env::var("BOT_GPU_COMPLETION_PROFILE").as_deref() == Ok("1") {
                host::render_profile::enable_gpu_completion();
            }
        }
        if std::env::var("BOT_RESPONSIVENESS_PROFILE").as_deref() == Ok("1") {
            host::responsiveness_profile::enable();
            if std::env::var("BOT_RESPONSIVENESS_FINE").as_deref() == Ok("1") {
                host::responsiveness_profile::enable_fine();
            }
            if frontend == "panel" {
                host::responsiveness_profile::set_input_surface(
                    host::responsiveness_profile::InputSurface::Panel,
                );
            } else if frontend == "tui" {
                // Headless memory harness has no terminal.draw path.
                host::responsiveness_profile::set_input_surface(
                    host::responsiveness_profile::InputSurface::TuiHeadless,
                );
            }
        }
        use vault::{Profile, ProfileSettings, Vault};

        let names = crate::mint_live_names(config.n);
        if frontend=="panel" {crate::nav_capture::enable(&names);}
        let diagnostics = std::env::var("BOT_MEMORY_DIAGNOSTICS").as_deref() == Ok("1");
        // Resolved once at harness setup (not re-read per sample/failure).
        let failure_capture =
            std::env::var("BOT_MEMORY_FAILURE_CAPTURE").as_deref() == Ok("1");
        if diagnostics {
            crate::memory_diagnostics::enable(&names);
        }
        let pass = crate::live_vault_passphrase();
        let dir = std::env::temp_dir().join(format!("274bot-memory-{}", names[0]));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join("vault");
        let mut vault = Vault::create(&path, &pass).map_err(|e| e.to_string())?;

        for (i, name) in names.iter().enumerate() {
            let mut settings = ProfileSettings::default();
            settings.auto_login = true;
            vault
                .upsert(Profile {
                    username: name.clone(),
                    password: name.clone(),
                    uid: 274_900_000 + i as i32,
                    settings,
                })
                .map_err(|e| e.to_string())?;
        }
        // Seeds install after Play exists (or via prepare's LoadDefault path).
        *SEEDS.lock().unwrap() = Some(HashMap::new());

        let card = if matches!(config.workload, Workload::Idle | Workload::SeededIdle) {
            None
        } else {
            let root = std::env::var_os("RS2B0T")
                .map(PathBuf::from)
                .ok_or("RS2B0T is required for the Thiever benchmark")?;
            let mut library = script::JsLibrary::new(dir.join("scripts.json"));
            library.register_rs2b0t(&root, &dir.join("rs2b0t-path"))?;
            library.ensure_js(script::ScriptSource::Catalog, "Thiever")?;
            let card = library
                .get(script::ScriptSource::Catalog, "Thiever")
                .cloned()
                .ok_or("missing Thiever")?;
            let scenario = scenario::get("thiever").ok_or("missing Thiever scenario")?;
            let inject = scenario::settings_inject_map(scenario.settings.script_settings_inject);
            let bag = script::settings_store::merge_bag(
                &card.settings_schema,
                &serde_json::Map::new(),
                inject.as_ref(),
            );
            let siblings = script::resolve_sibling_modules(
                &card.path,
                &card.origin,
                library.cache(),
                script::CacheMeta {
                    kind: card.kind,
                    source: card.source,
                    shape: None,
                },
            )?;
            Some((card, bag, siblings))
        };

        let output_path = std::env::var_os("BOT_MEMORY_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| dir.join("samples.jsonl"));
        let output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output_path)
            .map_err(|e| format!("{}: {e}", output_path.display()))?;
        let qualification_output = std::fs::OpenOptions::new()
            .write(true).create_new(true)
            .open(output_path.with_extension("qualification.jsonl"))
            .map_err(|e| e.to_string())?;
        let diagnostic_output = if diagnostics {
            Some(
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(output_path.with_extension("diagnostics.jsonl"))
                    .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };
        eprintln!(
            "memory benchmark {frontend}: {} slots {}; {}",
            config.n,
            config.workload.as_str(),
            output_path.display()
        );
        Ok(Self {
            config,
            names,
            vault: path,
            pass,
            frontend,
            started: Instant::now(),
            warm: None,
            observing: None,
            last_sample: None,
            lifecycle_cycle: 0,
            stopped: false,
            teardown: None,
            card,
            output,
            diagnostics,
            failure_capture,
            failure_boundary_attempted: false,
            diagnostic_output,
            qualification_output,
            single_renderer,
            render_policy,
        })
    }

    /// Install Thiever seed runners for non-idle workloads.
    ///
    /// [`SeedNav::FromPlay`] clones the Play-owned Arc (or keeps `None` when
    /// the pack failed) — no second `load_pack`. [`SeedNav::LoadDefault`]
    /// decodes once for unit tests without a Play. Unseeded idle leaves an
    /// empty seed map. Call before spawning slots so `client_frame` sees them.
    pub fn bind_seed_nav(&self, seed_nav: SeedNav) -> Result<(), String> {
        if self.config.workload == Workload::Idle {
            *SEEDS.lock().unwrap() = Some(HashMap::new());
            return Ok(());
        }
        let seed_world = match seed_nav {
            SeedNav::LoadDefault => nav::world::NavWorld::load_pack(&scenario::default_pack_path())
                .ok()
                .map(Arc::new),
            SeedNav::FromPlay(world) => world,
        };
        let mut seeds = HashMap::new();
        for name in &self.names {
            let mut scenario = if self.config.workload == Workload::SeededIdle {
                seeded_idle_scenario()
            } else if std::env::var("BOT_MEMORY_SUSTAIN").as_deref() == Ok("1") {
                scenario::thiever_sustained_scenario()
            } else {
                scenario::get("thiever").ok_or("missing Thiever scenario")?
            };
            scenario.settings.terminal_shot = None;
            let seed = seed_runner(scenario, name, seed_world.clone());
            seeds.insert(name.clone(), Arc::new(Mutex::new(seed)));
        }
        *SEEDS.lock().unwrap() = Some(seeds);
        Ok(())
    }

    pub fn has_script_card(&self) -> bool {
        self.card.is_some()
    }

    fn start_script(&self, play: &Play, name: &str) -> Result<(), String> {
        let Some((card, bag, siblings)) = &self.card else {
            return Ok(());
        };
        if std::env::var("BOT_MEMORY_SUSTAIN").as_deref() == Ok("1") {
            let mut bag = bag.clone();
            bag.insert("banking".into(), serde_json::json!("Auto"));
            bag.insert("loadout".into(), serde_json::json!("Memory food"));
            bag.insert("foodWithdraw".into(), serde_json::json!(22));
            bag.insert("bankAtFood".into(), serde_json::json!(3));
            let slot = crate::script_slot_or_insert(&play.scripts, name);
            let mut slot = slot.lock().unwrap();
            slot.start_load_with_loadouts(card.js.clone(), card.shape, siblings.clone(), &[script::Loadout { name: "Memory food".into(), worn: vec![], carry: vec!["Lobster".into()] }])?;
            slot.post_settings_bag(&bag);
            drop(slot);
            play.wake(name);
            return Ok(());
        }
        play.script_start_load(
            name,
            card.js.clone(),
            card.shape,
            Some(bag.clone()),
            siblings.clone(),
        )
    }

    /// Drive warmup/observe/lifecycle. `Ok(true)` when teardown finished.
    pub fn poll(&mut self, play: &Play) -> Result<bool, String> {
        let now = Instant::now();
        let statuses = play.statuses();
        let ready = statuses
            .iter()
            .filter(|s| s.ingame && s.scene_state == 2)
            .count();

        let mut seeded = 0usize;
        let mut proved = 0usize;
        if self.config.workload != Workload::Idle {
            let map = {
                let seeds = SEEDS.lock().unwrap();
                seeds
                    .as_ref()
                    .cloned()
                    .ok_or("memory seeds not installed")?
            };
            for name in &self.names {
                let seed_arc = map
                    .get(name)
                    .ok_or_else(|| format!("missing seed for {name}"))?
                    .clone();
                let (status, on_start, already_started) = {
                    let seed = seed_arc.lock().unwrap();
                    (
                        seed.runner.status(),
                        seed.runner.on_start_script(),
                        seed.started,
                    )
                };
                match status {
                    scenario::RunnerStatus::Failed(msg) => {
                        let failure =
                            format!("Thiever seed/proof failed for {name}: {msg}");
                        return self
                            .handle_harness_failure(
                                failure,
                                |run| run.qualification_slots(play),
                                |run, err| run.write_diagnostics(play, Some(err)),
                            )
                            .map(|never| match never {});
                    }
                    scenario::RunnerStatus::Passed => {
                        seeded += 1;
                        proved += 1;
                    }
                    _ if on_start => {
                        seeded += 1;
                        if !already_started {
                            self.start_script(play, name)?;
                            seed_arc.lock().unwrap().started = true;
                        }
                    }
                    _ => {}
                }
                if let Some(error) = play.script_last_error(name) {
                    let failure = format!("{name}: {error}");
                    return self
                        .handle_harness_failure(
                            failure,
                            |run| run.qualification_slots(play),
                            |run, err| run.write_diagnostics(play, Some(err)),
                        )
                        .map(|never| match never {});
                }
            }
        }

        let active = self
            .names
            .iter()
            .filter(|name| play.script_state(name) == script::RunState::Running)
            .count();

        // Unseeded idle: ready only. Seeded idle: seed completed, no scripts.
        // Active/lifecycle: all ready, seeded, XP-proved, scripts up.
        let established = if self.config.workload == Workload::SeededIdle {
            ready == self.config.n && seeded == self.config.n && active == 0
        } else if self.card.is_none() {
            ready == self.config.n
        } else {
            ready == self.config.n
                && seeded == self.config.n
                && proved == self.config.n
                && active == self.config.n
        };

        if self.warm.is_none() && established {
            self.warm = Some(now);
        }
        if self.warm.is_none() && self.started.elapsed() > Duration::from_secs(1800) {
            return Err(format!(
                "blocked: ready={ready} seeded={seeded} proved={proved} wanted={}",
                self.config.n
            ));
        }
        if self.observing.is_none() && self.warm.is_some_and(|t| t.elapsed() >= self.config.warmup)
        {
            if !established {
                return Err("workload did not remain ready through warmup".into());
            }
            self.write_qualification(play, "observe-start")?;
            self.observing = Some(now);
        }
        if let Some(observing) = self.observing {
            if self.config.workload == Workload::Lifecycle && self.teardown.is_none() {
                let cycle = observing.elapsed().as_secs() / 60;
                let should_stop = cycle % 2 == 1;
                if cycle != self.lifecycle_cycle {
                    for name in &self.names {
                        if should_stop {
                            play.script_stop(name);
                        } else {
                            self.start_script(play, name)?;
                        }
                    }
                    self.stopped = should_stop;
                    self.lifecycle_cycle = cycle;
                }
            }
            if observing.elapsed() >= self.config.observe && self.teardown.is_none() {
                self.write_qualification(play, "observe-end")?;
                for name in &self.names {
                    play.script_stop(name);
                }
                self.stopped = true;
                self.teardown = Some(now);
            }
        }

        if self
            .last_sample
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(1))
        {
            let (allocs, alloc_bytes, live_bytes) = rust_allocator_counts();
            let metrics = script::memory_profile::snapshots();
            let sum = |key:&str|metrics.iter().filter_map(|m|m[key].as_u64()).sum::<u64>();
            let live = sum("v8_live");
            let sampled = metrics.iter().filter(|m|m["v8_live"]==1 && m["v8_heap_samples"].as_u64().unwrap_or(0)>0).count() as u64;
            let gpu = client::profiling::gpu_bytes();
            // Stamp mono at the same Instant used for elapsed_s so the sibling
            // encloses the native phase boundary (not a later post-metrics time).
            let (elapsed_s, elapsed_mono_ns) = if host::responsiveness_profile::enabled() {
                let now_i = std::time::Instant::now();
                let elapsed_s = now_i
                    .saturating_duration_since(self.started)
                    .as_secs_f64();
                (
                    elapsed_s,
                    Some(host::responsiveness_profile::mono_ns(now_i)),
                )
            } else {
                (self.started.elapsed().as_secs_f64(), None)
            };
            let sample = Sample {
                frontend: self.frontend.into(),
                n: self.config.n,
                workload: self.config.workload,
                elapsed_s,
                phase: if self.teardown.is_some() {
                    "teardown".into()
                } else if self.observing.is_some() {
                    "observe".into()
                } else if self.warm.is_some() {
                    "warmup".into()
                } else {
                    "seed".into()
                },
                ready,
                active,
                resident_bytes: crate::current_resident_bytes(),
                peak_resident_bytes: Some(crate::sample_process().0),
                rust_allocations: (!cfg!(feature = "memory-profile-no-alloc")).then_some(allocs),
                rust_allocated_bytes: (!cfg!(feature = "memory-profile-no-alloc")).then_some(alloc_bytes),
                rust_live_bytes: (!cfg!(feature = "memory-profile-no-alloc")).then_some(live_bytes),
                snapshot_inflight_bytes: Some(sum("snapshot_inflight_bytes")),
                snapshot_inflight_capacity: Some(sum("snapshot_inflight_capacity")),
                v8_used_bytes: (live==sampled).then(||sum("v8_used_bytes")),
                v8_total_bytes: (live==sampled).then(||sum("v8_total_bytes")),
                gpu_tracked_bytes: Some(gpu.buffers+gpu.textures),
            };
            let mut value = sample.to_json();
            let cpu = process_cpu_seconds();
            value["process_cpu_user_s"] = cpu.map(|v| v.0).into();
            value["process_cpu_system_s"] = cpu.map(|v| v.1).into();
            value["allocation_counting"] = (!cfg!(feature = "memory-profile-no-alloc")).into();
            value["diagnostic_sidecar"] = self.diagnostics.into();
            value["failure_capture"] = self.failure_capture.into();
            // Requested mode metadata only — not observed GPU/cadence proof.
            value["single_renderer"] = self.single_renderer.into();
            value["render_policy"] = self.render_policy.as_str().into();
            value["render_policy_requested"] = true.into();
            value["gpu_buffer_bytes"] = gpu.buffers.into();
            value["gpu_texture_bytes"] = gpu.textures.into();
            value["gpu_peak_tracked_bytes"] = gpu.peak.into();
            value["v8_live_isolates"] = live.into();
            value["v8_sampled_isolates"] = sampled.into();
            let now_ms=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
            value["v8_max_sample_age_ms"] = metrics.iter().filter(|m|m["v8_live"]==1).filter_map(|m|m["v8_updated_ms"].as_u64()).map(|t|now_ms.saturating_sub(t)).max().map(serde_json::Value::from).unwrap_or(serde_json::Value::Null);
            for key in ["script_tick_count","script_tick_total_ns"] {value[key]=sum(key).into();}
            value["script_tick_max_ns"]=metrics.iter().filter_map(|m|m["script_tick_max_ns"].as_u64()).max().unwrap_or(0).into();
            value["snapshot_sum_isolate_peak_capacity"] = sum("snapshot_peak_capacity").into();
            for (key,counter) in [("client_tick",&client::profiling::CLIENT_TICK),("ui_draw",&client::profiling::UI_DRAW),("ui_frame",&client::profiling::UI_FRAME)] {
                let (count,total,max)=counter.read();
                value[format!("{key}_count")]=count.into();value[format!("{key}_total_ns")]=total.into();value[format!("{key}_max_ns")]=max.into();
            }
            // Legacy process-wide drawing/non-drawing groups — array schema unchanged.
            value["scheduling"] = host::cadence::read().map(|groups| {
                serde_json::Value::Array(groups.iter().enumerate().map(|(i,c)| serde_json::json!({
                    "drawing": i==1, "cycles":c.cycles, "work_ns":c.work_ns,
                    "requested_sleep_ns":c.requested_sleep_ns,"actual_sleep_ns":c.actual_sleep_ns,
                    "work_overruns":c.work_overruns,"intervals":c.intervals,"interval_ns":c.interval_ns,
                    "sleep_excess_buckets":c.sleep_excess,"interval_excess_buckets":c.interval_excess
                })).collect())
            }).unwrap_or(serde_json::Value::Null);
            // Per-slot absolute start-to-start intervals (sibling field; null while off).
            // Built in two json! layers so the field count stays under the crate
            // serde_json recursion limit (same pattern as renderer_profile).
            value["scheduling_slots"] = host::cadence::read_slots()
                .map(|slots| {
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);
                    serde_json::Value::Array(
                        slots
                            .into_iter()
                            .map(|s| {
                                let age = if s.updated_ms == 0 {
                                    serde_json::Value::Null
                                } else {
                                    serde_json::Value::from(now_ms.saturating_sub(s.updated_ms))
                                };
                                let mut row = serde_json::json!({
                                    "slot_id": s.slot_id,
                                    "generation": s.generation,
                                    "updated_ms": s.updated_ms,
                                    "sample_age_ms": age,
                                    "ended": s.ended,
                                    "drawing": s.drawing,
                                    "cycle_n": s.cycle_n,
                                    "drawing_cycle_n": s.drawing_cycle_n,
                                    "non_drawing_cycle_n": s.non_drawing_cycle_n,
                                    "work_ns": s.work_ns,
                                    "requested_sleep_ns": s.requested_sleep_ns,
                                    "actual_sleep_ns": s.actual_sleep_ns,
                                    "work_overrun_n": s.work_overrun_n,
                                    "interval_n": s.interval_n,
                                    "interval_ns": s.interval_ns,
                                    "interval_buckets": s.interval_buckets.as_slice(),
                                    "interval_bound_ms": host::cadence::INTERVAL_BOUNDS_MS.as_slice(),
                                    "interval_p99_upper_bound_ms": host::cadence::p99_upper_bound_ms(&s.interval_buckets),
                                    "interval_coverage_complete": host::cadence::interval_coverage_complete(&s),
                                    "mode_break_n": s.mode_break_n,
                                    "park_n": s.park_n,
                                    "anchor_miss_n": s.anchor_miss_n,
                                    "first_interval_ms": s.first_interval_ms,
                                    "last_interval_ms": s.last_interval_ms,
                                    "first_interval_start_mono_ms": s.first_interval_start_mono_ms,
                                    "last_interval_end_mono_ms": s.last_interval_end_mono_ms,
                                    "first_cycle_ms": s.first_cycle_ms,
                                    "last_cycle_ms": s.last_cycle_ms,
                                    "first_cycle_mono_ms": s.first_cycle_mono_ms,
                                    "last_cycle_mono_ms": s.last_cycle_mono_ms,
                                    "sleep_excess_buckets": s.sleep_excess,
                                    "interval_excess_buckets": s.interval_excess,
                                    "ended_lost_n": host::cadence::ended_lost_n(),
                                });
                                if let Some(obj) = row.as_object_mut() {
                                    obj.insert(
                                        "flush_lag_max_cycles".into(),
                                        host::cadence::FLUSH_LAG_MAX_CYCLES.into(),
                                    );
                                    obj.insert(
                                        "flush_lag_max_ms".into(),
                                        host::cadence::FLUSH_LAG_MAX_MS.into(),
                                    );
                                    obj.insert(
                                        "scene_transition_separation".into(),
                                        host::cadence::SCENE_TRANSITION_SEPARATION.into(),
                                    );
                                    obj.insert(
                                        "interval_endpoint_means".into(),
                                        "tick_start_instant_wall_backdated_and_local_mono_not_post_sleep_record".into(),
                                    );
                                    obj.insert(
                                        "updated_ms_means".into(),
                                        "registry_publish_freshness_not_sample_endpoint".into(),
                                    );
                                    obj.insert(
                                        "jsonl_row_time_is_not_interval_endpoint".into(),
                                        true.into(),
                                    );
                                    obj.insert(
                                        "observe_window_means".into(),
                                        "use_counter_deltas_with_endpoint_stamps_never_nominal_jsonl_delta_t".into(),
                                    );
                                    obj.insert(
                                        "interval_means".into(),
                                        "tick_start_to_start_same_drawing_no_park_absolute_not_excess".into(),
                                    );
                                    obj.insert(
                                        "mode_break_means".into(),
                                        "drawing_latch_flip_while_anchor_present_excluded_from_intervals".into(),
                                    );
                                    obj.insert(
                                        "park_means".into(),
                                        "idle_park_cleared_start_anchor_excluded_from_intervals".into(),
                                    );
                                }
                                row
                            })
                            .collect(),
                    )
                })
                .unwrap_or(serde_json::Value::Null);
            // Host mainredraw/paint completions and live renderer residency —
            // not GPU completed/presented frames. Null while profiling off.
            // GPU queue-completion (nested object) is built separately to keep
            // serde_json::json! under the crate recursion limit.
            value["renderer_profile"] = host::render_profile::read()
                .map(|slots| {
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);
                    let gpu_on = host::render_profile::gpu_completion_enabled();
                    serde_json::Value::Array(
                        slots
                            .into_iter()
                            .map(|s| {
                                let age = if s.updated_ms == 0 {
                                    serde_json::Value::Null
                                } else {
                                    serde_json::Value::from(now_ms.saturating_sub(s.updated_ms))
                                };
                                let mut row = serde_json::json!({
                                    "slot_id": s.slot_id,
                                    "generation": s.generation,
                                    "renderer_present": s.renderer_present,
                                    "backend_kind": s.backend.as_str(),
                                    "prefer_cpu": s.prefer_cpu,
                                    "ingame": s.ingame,
                                    "scene_state": s.scene_state,
                                    "draw": s.draw,
                                    "full_rate": s.full_rate,
                                    "client_loop_n": s.client_loop_n,
                                    "paint_n": s.paint_n,
                                    "skip_n": s.skip_n,
                                    "stable_paint_n": s.stable_paint_n,
                                    "transition_paint_n": s.transition_paint_n,
                                    "stable_paint_intervals": s.stable_paint_intervals,
                                    "stable_paint_interval_ns": s.stable_paint_interval_ns,
                                    "stable_paint_interval_buckets": s.stable_paint_interval_buckets,
                                    "transition_paint_intervals": s.transition_paint_intervals,
                                    "transition_paint_interval_ns": s.transition_paint_interval_ns,
                                    "transition_paint_interval_buckets": s.transition_paint_interval_buckets,
                                    "interval_bound_ms": host::render_profile::INTERVAL_BOUNDS_MS,
                                    "attach_n": s.attach_n,
                                    "detach_n": s.detach_n,
                                    "backend_change_n": s.backend_change_n,
                                    "updated_ms": s.updated_ms,
                                    "sample_age_ms": age,
                                    "ended": s.ended,
                                    "paint_count_means": "host_mainredraw_completions_not_gpu_presented_frames",
                                });
                                // CPU delivery after prior submit — not HW GPU ts or scanout.
                                row["gpu_completion"] = serde_json::json!({
                                    "enabled": gpu_on,
                                    "gpu_frame_n": s.gpu_frame_n,
                                    "cpu_frame_n": s.cpu_frame_n,
                                    "registered_n": s.gpu_registered_n,
                                    "completed_n": s.gpu_completed_n,
                                    "dropped_n": s.gpu_dropped_n,
                                    "lost_n": s.gpu_lost_n,
                                    "pending_n": s.gpu_pending_n,
                                    "oldest_pending_age_ms": s.gpu_oldest_pending_age_ms,
                                    "stable_completed_n": s.gpu_stable_completed_n,
                                    "transition_completed_n": s.gpu_transition_completed_n,
                                    "completion_latency_n": s.gpu_completion_latency_n,
                                    "completion_latency_ns": s.gpu_completion_latency_ns,
                                    "completion_latency_buckets": s.gpu_completion_latency_buckets,
                                    "stable_completion_intervals": s.gpu_stable_completion_intervals,
                                    "stable_completion_interval_ns": s.gpu_stable_completion_interval_ns,
                                    "stable_completion_interval_buckets": s.gpu_stable_completion_interval_buckets,
                                    "transition_completion_intervals": s.gpu_transition_completion_intervals,
                                    "transition_completion_interval_ns": s.gpu_transition_completion_interval_ns,
                                    "transition_completion_interval_buckets": s.gpu_transition_completion_interval_buckets,
                                    "interval_bound_ms": host::render_profile::INTERVAL_BOUNDS_MS,
                                    "registration_complete": host::render_profile::gpu_registration_complete(&s),
                                    "completion_coverage_complete": host::render_profile::gpu_completion_coverage_complete(&s),
                                    "timestamp_semantics": "callback_delivery_cpu_after_prior_submit_not_hw_gpu_or_scanout",
                                    "latency_means": "mainredraw_start_to_callback_delivery_upper_bound_not_pre_paint_or_scanout",
                                    "interval_means": "observed_callback_delivery_cadence_same_mode_epoch_only",
                                });
                                row
                            })
                            .collect(),
                    )
                })
                .unwrap_or(serde_json::Value::Null);
            // Decode→script dispatch and focused-input→UI endpoint latencies.
            // Null while profiling off. Input endpoint is surface-specific;
            // display scanout remains explicitly unavailable on panel.
            // Sample/read mono brackets share CLOCK_DOMAIN with per-row capture
            // brackets; wall updated_ms/sample_age_ms remain legacy siblings.
            value["responsiveness_profile"] = host::responsiveness_profile::read_bracketed()
                .map(|snap| {
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);
                    let ack = host::responsiveness_profile::visible_ack_status();
                    let surface = host::responsiveness_profile::input_surface().as_str();
                    let fine_on = host::responsiveness_profile::fine_enabled();
                    let elapsed_ns = elapsed_mono_ns.unwrap_or(snap.read_mono_ns_lower);
                    let sample_lo = elapsed_ns.min(snap.read_mono_ns_lower);
                    let sample_hi = snap.read_mono_ns_upper.max(elapsed_ns).max(sample_lo);
                    value["responsiveness_clock"] = serde_json::json!({
                        "domain": host::responsiveness_profile::CLOCK_DOMAIN,
                        "elapsed_mono_ns": elapsed_mono_ns,
                        "elapsed_mono_ms_lower": elapsed_mono_ns.map(host::responsiveness_profile::mono_ns_floor_ms),
                        "elapsed_mono_ms_upper": elapsed_mono_ns.map(host::responsiveness_profile::mono_ns_ceil_ms),
                        "read_mono_ns_lower": snap.read_mono_ns_lower,
                        "read_mono_ns_upper": snap.read_mono_ns_upper,
                        "read_mono_ms_lower": snap.read_mono_ms_lower(),
                        "read_mono_ms_upper": snap.read_mono_ms_upper(),
                        "sample_mono_ns_lower": sample_lo,
                        "sample_mono_ns_upper": sample_hi,
                        "sample_mono_ms_lower": host::responsiveness_profile::mono_ns_floor_ms(sample_lo),
                        "sample_mono_ms_upper": host::responsiveness_profile::mono_ns_ceil_ms(sample_hi),
                        "fine_latency_enabled": fine_on,
                        "means": "process_local_mono_ns_enclosing_elapsed_capture_and_registry_read_floor_ceil_ms_siblings_not_cross_run",
                    });
                    serde_json::Value::Array(
                        snap.slots
                            .into_iter()
                            .map(|s| {
                                let age = if s.updated_ms == 0 {
                                    serde_json::Value::Null
                                } else {
                                    serde_json::Value::from(now_ms.saturating_sub(s.updated_ms))
                                };
                                // Two-layer json! (core row + fine/ack siblings) stays under the
                                // serde_json recursion limit — same pattern as scheduling_slots.
                                let mut row = serde_json::json!({
                                    "slot_id": s.slot_id,
                                    "generation": s.generation,
                                    "updated_ms": s.updated_ms,
                                    "sample_age_ms": age,
                                    "decode_capture_mono_ns_lower": s.decode_capture_mono_ns_lower,
                                    "decode_capture_mono_ns_upper": s.decode_capture_mono_ns_upper,
                                    "decode_capture_mono_ms_lower": host::responsiveness_profile::mono_ns_floor_ms(s.decode_capture_mono_ns_lower),
                                    "decode_capture_mono_ms_upper": host::responsiveness_profile::mono_ns_ceil_ms(s.decode_capture_mono_ns_upper),
                                    "input_capture_mono_ns_lower": s.input_capture_mono_ns_lower,
                                    "input_capture_mono_ns_upper": s.input_capture_mono_ns_upper,
                                    "input_capture_mono_ms_lower": host::responsiveness_profile::mono_ns_floor_ms(s.input_capture_mono_ns_lower),
                                    "input_capture_mono_ms_upper": host::responsiveness_profile::mono_ns_ceil_ms(s.input_capture_mono_ns_upper),
                                    "ended": s.ended,
                                    "input_surface": surface,
                                    "decode_edge_n": s.decode_edge_n,
                                    "dispatch_n": s.dispatch_n,
                                    "decode_canceled_n": s.decode_canceled_n,
                                    "decode_unmatched_canceled_n": s.decode_unmatched_canceled_n,
                                    "decode_lost_n": s.decode_lost_n,
                                    "decode_dropped_n": s.decode_dropped_n,
                                    "decode_pending_n": s.decode_pending_n,
                                    "decode_latency_n": s.decode_latency_n,
                                    "decode_latency_ns": s.decode_latency_ns,
                                    "decode_latency_buckets": s.decode_latency_buckets,
                                    "decode_p99_upper_bound_ms": host::responsiveness_profile::p99_upper_bound_ms(&s.decode_latency_buckets),
                                    "decode_coverage_complete": host::responsiveness_profile::decode_coverage_complete(&s),
                                    "decode_accounting_exact": host::responsiveness_profile::decode_accounting_exact(&s),
                                    "decode_means": "PLAYER_INFO_after_drain_to_on_game_tick_entry",
                                    "input_start_n": s.input_start_n,
                                    "input_complete_n": s.input_complete_n,
                                    "input_canceled_n": s.input_canceled_n,
                                    "input_lost_n": s.input_lost_n,
                                    "input_dropped_n": s.input_dropped_n,
                                    "input_pending_n": s.input_pending_n,
                                    "input_latency_n": s.input_latency_n,
                                    "input_latency_ns": s.input_latency_ns,
                                    "input_latency_buckets": s.input_latency_buckets,
                                    "input_p99_upper_bound_ms": host::responsiveness_profile::p99_upper_bound_ms(&s.input_latency_buckets),
                                    "input_coverage_complete": host::responsiveness_profile::input_coverage_complete(&s),
                                    "latency_bound_ms": host::responsiveness_profile::LATENCY_BOUNDS_MS,
                                });
                                if let Some(obj) = row.as_object_mut() {
                                    if fine_on {
                                        obj.insert(
                                            "decode_fine_latency_buckets".into(),
                                            serde_json::json!(s.decode_fine_latency_buckets.as_slice()),
                                        );
                                        obj.insert(
                                            "decode_fine_p99_upper_bound_ms".into(),
                                            serde_json::json!(
                                                host::responsiveness_profile::fine_p99_upper_bound_ms(
                                                    &s.decode_fine_latency_buckets
                                                )
                                            ),
                                        );
                                        obj.insert(
                                            "input_fine_latency_buckets".into(),
                                            serde_json::json!(s.input_fine_latency_buckets.as_slice()),
                                        );
                                        obj.insert(
                                            "input_fine_p99_upper_bound_ms".into(),
                                            serde_json::json!(
                                                host::responsiveness_profile::fine_p99_upper_bound_ms(
                                                    &s.input_fine_latency_buckets
                                                )
                                            ),
                                        );
                                        obj.insert(
                                            "fine_latency_bound_ms".into(),
                                            serde_json::json!(
                                                host::responsiveness_profile::FINE_LATENCY_BOUNDS_MS
                                                    .as_slice()
                                            ),
                                        );
                                    } else {
                                        obj.insert(
                                            "decode_fine_latency_buckets".into(),
                                            serde_json::Value::Null,
                                        );
                                        obj.insert(
                                            "decode_fine_p99_upper_bound_ms".into(),
                                            serde_json::Value::Null,
                                        );
                                        obj.insert(
                                            "input_fine_latency_buckets".into(),
                                            serde_json::Value::Null,
                                        );
                                        obj.insert(
                                            "input_fine_p99_upper_bound_ms".into(),
                                            serde_json::Value::Null,
                                        );
                                        obj.insert(
                                            "fine_latency_bound_ms".into(),
                                            serde_json::Value::Null,
                                        );
                                    }
                                    obj.insert(
                                        "visible_ack".into(),
                                        serde_json::json!({
                                            "available": ack.available,
                                            "endpoint": ack.endpoint,
                                            "missing_capability": ack.missing_capability,
                                            "means": ack.means,
                                        }),
                                    );
                                }
                                row
                            })
                            .collect(),
                    )
                })
                .unwrap_or_else(|| {
                    value["responsiveness_clock"] = serde_json::Value::Null;
                    serde_json::Value::Null
                });
            writeln!(self.output, "{value}").map_err(|e| e.to_string())?;
            if self.diagnostics {
                self.write_diagnostics(play, None)?;
            }
            self.last_sample = Some(now);
        }

        Ok(self
            .teardown
            .is_some_and(|t| t.elapsed() >= Duration::from_secs(60)))
    }

    // Two boundary reads preserve script progress evidence when verbose
    // diagnostic collection is disabled. Never drains logs or sends actions.
    // Slot reads run before elapsed_s so observe-boundary timing matches the
    // pre-failure-capture path (flag off / success rows unchanged).
    fn qualification_slots(&self, play: &Play) -> Vec<serde_json::Value> {
        let statuses = play.statuses();
        self.names
            .iter()
            .map(|name| {
                serde_json::json!({
                    "name": name,
                    "state": format!("{:?}", play.script_state(name)),
                    "error": play.script_last_error(name),
                    "runtime": play.memory_script_progress(name),
                    "client": statuses.iter().find(|s| &s.username == name).map(|s| serde_json::json!({
                        "ingame": s.ingame,
                        "scene_state": s.scene_state,
                        "x": s.tile_x,
                        "z": s.tile_z,
                        "level": s.tile_level
                    })),
                })
            })
            .collect()
    }

    fn write_qualification(&mut self, play: &Play, phase: &str) -> Result<(), String> {
        let slots = self.qualification_slots(play);
        let value = serde_json::json!({
            "phase": phase,
            "elapsed_s": self.started.elapsed().as_secs_f64(),
            "slots": slots
        });
        writeln!(self.qualification_output, "{value}").map_err(|e| e.to_string())
    }

    /// Returns true the first time failure-capture should attempt a row.
    fn arm_failure_boundary_once(&mut self) -> bool {
        if !self.failure_capture || self.failure_boundary_attempted {
            return false;
        }
        self.failure_boundary_attempted = true;
        true
    }

    /// Production seed/script failure exit used by `poll`/`advance` and unit tests.
    ///
    /// Flag contract:
    /// - Latch **before** the slots producer and I/O. Disabled or already-latched
    ///   paths never call `slots` (no cached state copies/locks).
    /// - `failure_capture`: at most one best-effort qualification row; write
    ///   failures never replace `failure`.
    /// - `diagnostics`: `diag` runs only when on. If `failure_capture` is also
    ///   on, diag write errors are swallowed so the original `failure` is kept.
    ///   If `failure_capture` is off, legacy `?` semantics apply (diag Err wins).
    /// - neither on: returns `failure` with no producer/I/O.
    fn handle_harness_failure<S, D>(
        &mut self,
        failure: String,
        slots: S,
        diag: D,
    ) -> Result<std::convert::Infallible, String>
    where
        S: FnOnce(&Self) -> Vec<serde_json::Value>,
        D: FnOnce(&mut Self, &str) -> Result<(), String>,
    {
        if self.arm_failure_boundary_once() {
            let rows = slots(self);
            let value = serde_json::json!({
                "phase": "failure-boundary",
                "elapsed_s": self.started.elapsed().as_secs_f64(),
                "slots": rows,
                "record": "failure-boundary",
                "failure": &failure,
            });
            let _ = writeln!(self.qualification_output, "{value}");
        }
        if self.diagnostics {
            let r = diag(self, &failure);
            if !self.failure_capture {
                r?;
            }
        }
        Err(failure)
    }

    fn write_diagnostics(&mut self, play: &Play, failure: Option<&str>) -> Result<(), String> {
        let mut slots = Vec::with_capacity(self.names.len());
        let seeds = SEEDS.lock().unwrap();
        for name in &self.names {
            let seed = seeds.as_ref().and_then(|s| s.get(name)).map(|s| {
                let s = s.lock().unwrap();
                serde_json::json!({"status":format!("{:?}",s.runner.status()),"started":s.started})
            });
            slots.push(serde_json::json!({"name":name,"seed":seed,"state":format!("{:?}",play.script_state(name)),"error":play.script_last_error(name),"runtime":play.memory_script_progress(name),"diagnostics":crate::memory_diagnostics::sample(name)}));
        }
        let row = serde_json::json!({"record":"diagnostics","elapsed_s":self.started.elapsed().as_secs_f64(),"failure":failure,"slots":slots});
        // A separate record type in the diagnostic-only sidecar keeps original
        // Sample consumers and baseline JSONL unchanged.
        writeln!(
            self.diagnostic_output.as_mut().expect("diagnostic output"),
            "{row}"
        )
        .map_err(|e| e.to_string())
    }

    /// Rotate focus every 30s, or keep slot zero for fixed-focus panel cells.
    pub fn focus_index(&self) -> usize {
        if self.render_policy.pins_focus() {
            return 0;
        }
        let n = self.names.len().max(1);
        (self.started.elapsed().as_secs() / 30) as usize % n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Env mutation is process-global; serialize these tests.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Recover from a prior poisoned lock so one panicking test does not
    /// cascade through the whole env-serialized suite.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    struct EnvGuard {
        keys: Vec<&'static str>,
    }

    impl EnvGuard {
        fn clear(keys: &[&'static str]) -> Self {
            for k in keys {
                std::env::remove_var(k);
            }
            Self {
                keys: keys.to_vec(),
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for k in &self.keys {
                std::env::remove_var(k);
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn process_cpu_time_is_available_and_monotonic() {
        let before = process_cpu_seconds().expect("getrusage");
        let after = process_cpu_seconds().expect("getrusage");
        assert!(before.0 >= 0.0 && before.1 >= 0.0);
        assert!(after.0 >= before.0 && after.1 >= before.1);
    }

    #[test]
    fn parse_n_accepts_1_16_32_128() {
        assert_eq!(parse_n("1").unwrap(), 1);
        assert_eq!(parse_n("16").unwrap(), 16);
        assert_eq!(parse_n("32").unwrap(), 32);
        assert_eq!(parse_n("128").unwrap(), 128);
    }

    #[test]
    fn parse_n_rejects_invalid() {
        for s in ["0", "2", "50", "", "-1"] {
            assert!(parse_n(s).is_err(), "expected err for {s:?}");
        }
    }

    fn clear_render_env() -> EnvGuard {
        EnvGuard::clear(&[
            "BOT_MEMORY_SINGLE_RENDERER",
            "BOT_MEMORY_RENDER_POLICY",
        ])
    }

    #[test]
    fn parse_render_policy_defaults_rotating_all() {
        let _lock = env_lock();
        let _g = clear_render_env();
        assert_eq!(
            parse_render_policy("panel").unwrap(),
            RenderPolicy::RotatingAll
        );
        assert_eq!(
            parse_render_policy("tui").unwrap(),
            RenderPolicy::RotatingAll
        );
    }

    #[test]
    fn parse_render_policy_legacy_single_renderer() {
        let _lock = env_lock();
        let _g = clear_render_env();
        std::env::set_var("BOT_MEMORY_SINGLE_RENDERER", "1");
        assert_eq!(
            parse_render_policy("panel").unwrap(),
            RenderPolicy::FixedOne
        );
        assert!(parse_render_policy("tui").is_err());
    }

    #[test]
    fn parse_render_policy_explicit_modes() {
        let _lock = env_lock();
        let _g = clear_render_env();
        std::env::set_var("BOT_MEMORY_RENDER_POLICY", "focused-one");
        assert_eq!(
            parse_render_policy("panel").unwrap(),
            RenderPolicy::FocusedOne
        );
        std::env::set_var("BOT_MEMORY_RENDER_POLICY", "focused-plus-background");
        assert_eq!(
            parse_render_policy("panel").unwrap(),
            RenderPolicy::FocusedPlusBackground
        );
        assert!(parse_render_policy("tui").is_err());
    }

    #[test]
    fn parse_render_policy_rejects_conflict_and_unknown() {
        let _lock = env_lock();
        let _g = clear_render_env();
        std::env::set_var("BOT_MEMORY_SINGLE_RENDERER", "1");
        std::env::set_var("BOT_MEMORY_RENDER_POLICY", "focused-one");
        assert!(parse_render_policy("panel").is_err());
        std::env::remove_var("BOT_MEMORY_SINGLE_RENDERER");
        std::env::set_var("BOT_MEMORY_RENDER_POLICY", "nope");
        assert!(parse_render_policy("panel").is_err());
    }

    #[test]
    fn prepare_reads_focused_one_policy_and_pins_focus() {
        let _lock = env_lock();
        let _g = EnvGuard::clear(&[
            "RS2B0T",
            "BOT_MEMORY_OUTPUT",
            "BOT_MEMORY_SINGLE_RENDERER",
            "BOT_MEMORY_RENDER_POLICY",
        ]);
        std::env::set_var("BOT_MEMORY_RENDER_POLICY", "focused-one");
        let run = Run::prepare(unit_config(1, Workload::Idle), "panel").expect("prepare");
        assert_eq!(run.render_policy, RenderPolicy::FocusedOne);
        assert!(!run.single_renderer);
        assert_eq!(run.focus_index(), 0);
    }

    #[test]
    fn config_from_env_accepts_n16() {
        let _lock = env_lock();
        let _g = EnvGuard::clear(&[
            "BOT_MEMORY_N",
            "BOT_MEMORY_WORKLOAD",
            "BOT_MEMORY_WARMUP_S",
            "BOT_MEMORY_OBSERVE_S",
        ]);
        std::env::set_var("BOT_MEMORY_N", "16");
        let cfg = Config::from_env().unwrap().expect("Some");
        assert_eq!(cfg.n, 16);
    }

    #[test]
    fn prepare_background_policy_pins_focus() {
        let _lock = env_lock();
        let _g = EnvGuard::clear(&[
            "RS2B0T",
            "BOT_MEMORY_OUTPUT",
            "BOT_MEMORY_SINGLE_RENDERER",
            "BOT_MEMORY_RENDER_POLICY",
        ]);
        std::env::set_var("BOT_MEMORY_RENDER_POLICY", "focused-plus-background");
        let run = Run::prepare(unit_config(1, Workload::Idle), "panel").expect("prepare");
        assert_eq!(run.render_policy, RenderPolicy::FocusedPlusBackground);
        assert!(!run.single_renderer);
        assert_eq!(run.focus_index(), 0);
    }

    #[test]
    fn prepare_legacy_single_renderer_keeps_flag() {
        let _lock = env_lock();
        let _g = EnvGuard::clear(&[
            "RS2B0T",
            "BOT_MEMORY_OUTPUT",
            "BOT_MEMORY_SINGLE_RENDERER",
            "BOT_MEMORY_RENDER_POLICY",
        ]);
        std::env::set_var("BOT_MEMORY_SINGLE_RENDERER", "1");
        let run = Run::prepare(unit_config(1, Workload::Idle), "panel").expect("prepare");
        assert!(run.single_renderer);
        assert_eq!(run.render_policy, RenderPolicy::FixedOne);
        assert_eq!(run.focus_index(), 0);
    }

    #[test]
    fn prepare_tui_rejects_panel_render_policy() {
        let _lock = env_lock();
        let _g = EnvGuard::clear(&[
            "RS2B0T",
            "BOT_MEMORY_OUTPUT",
            "BOT_MEMORY_SINGLE_RENDERER",
            "BOT_MEMORY_RENDER_POLICY",
        ]);
        std::env::set_var("BOT_MEMORY_RENDER_POLICY", "focused-plus-background");
        let err = match Run::prepare(unit_config(1, Workload::Idle), "tui") {
            Err(e) => e,
            Ok(_) => panic!("expected panel-only error"),
        };
        assert!(
            err.contains("panel frontend"),
            "expected panel-only error, got {err}"
        );
    }

    #[test]
    fn config_from_env_none_without_bot_memory_n() {
        let _lock = env_lock();
        let _g = EnvGuard::clear(&[
            "BOT_MEMORY_N",
            "BOT_MEMORY_WORKLOAD",
            "BOT_MEMORY_WARMUP_S",
            "BOT_MEMORY_OBSERVE_S",
        ]);
        assert_eq!(Config::from_env().unwrap(), None);
    }

    #[test]
    fn config_from_env_rejects_invalid_n() {
        let _lock = env_lock();
        let _g = EnvGuard::clear(&[
            "BOT_MEMORY_N",
            "BOT_MEMORY_WORKLOAD",
            "BOT_MEMORY_WARMUP_S",
            "BOT_MEMORY_OBSERVE_S",
        ]);
        std::env::set_var("BOT_MEMORY_N", "2");
        assert!(Config::from_env().is_err());
    }

    #[test]
    fn config_from_env_rejects_invalid_workload() {
        let _lock = env_lock();
        let _g = EnvGuard::clear(&[
            "BOT_MEMORY_N",
            "BOT_MEMORY_WORKLOAD",
            "BOT_MEMORY_WARMUP_S",
            "BOT_MEMORY_OBSERVE_S",
        ]);
        std::env::set_var("BOT_MEMORY_N", "1");
        std::env::set_var("BOT_MEMORY_WORKLOAD", "sprint");
        assert!(Config::from_env().is_err());
    }

    #[test]
    fn config_from_env_rejects_zero_durations() {
        let _lock = env_lock();
        let _g = EnvGuard::clear(&[
            "BOT_MEMORY_N",
            "BOT_MEMORY_WORKLOAD",
            "BOT_MEMORY_WARMUP_S",
            "BOT_MEMORY_OBSERVE_S",
        ]);
        std::env::set_var("BOT_MEMORY_N", "1");
        std::env::set_var("BOT_MEMORY_WARMUP_S", "0");
        assert!(Config::from_env().is_err());
        std::env::remove_var("BOT_MEMORY_WARMUP_S");
        std::env::set_var("BOT_MEMORY_OBSERVE_S", "0");
        assert!(Config::from_env().is_err());
    }

    #[test]
    fn config_from_env_defaults_idle_and_durations() {
        let _lock = env_lock();
        let _g = EnvGuard::clear(&[
            "BOT_MEMORY_N",
            "BOT_MEMORY_WORKLOAD",
            "BOT_MEMORY_WARMUP_S",
            "BOT_MEMORY_OBSERVE_S",
        ]);
        std::env::set_var("BOT_MEMORY_N", "32");
        let cfg = Config::from_env().unwrap().expect("Some");
        assert_eq!(cfg.n, 32);
        assert_eq!(cfg.workload, Workload::Idle);
        assert_eq!(cfg.warmup.as_secs(), 120);
        assert_eq!(cfg.observe.as_secs(), 600);
    }

    #[test]
    fn sample_json_has_sibling_metric_fields_not_one_sum() {
        let sample = Sample {
            frontend: "unit".into(),
            n: 1,
            workload: Workload::Idle,
            elapsed_s: 1.5,
            phase: "observe".into(),
            ready: 1,
            active: 0,
            resident_bytes: Some(100),
            peak_resident_bytes: Some(200),
            rust_allocations: Some(3),
            rust_allocated_bytes: Some(40),
            rust_live_bytes: Some(30),
            snapshot_inflight_bytes: Some(10),
            snapshot_inflight_capacity: Some(20),
            v8_used_bytes: Some(50),
            v8_total_bytes: Some(60),
            gpu_tracked_bytes: Some(70),
        };
        let v = sample.to_json();
        let obj = v.as_object().expect("object");
        for key in [
            "frontend",
            "n",
            "workload",
            "elapsed_s",
            "phase",
            "ready",
            "active",
            "resident_bytes",
            "peak_resident_bytes",
            "rust_allocations",
            "rust_allocated_bytes",
            "rust_live_bytes",
            "snapshot_inflight_bytes",
            "snapshot_inflight_capacity",
            "v8_used_bytes",
            "v8_total_bytes",
            "gpu_tracked_bytes",
        ] {
            assert!(obj.contains_key(key), "missing sibling field {key}");
        }
        // Not one summed byte count: the distinct metric keys stay separate.
        assert!(obj.get("total_bytes").is_none());
        assert!(obj.get("bytes").is_none());
        assert_ne!(
            obj.get("resident_bytes").and_then(|x| x.as_u64()),
            obj.get("peak_resident_bytes").and_then(|x| x.as_u64())
        );
        assert_ne!(
            obj.get("rust_live_bytes").and_then(|x| x.as_u64()),
            obj.get("resident_bytes").and_then(|x| x.as_u64())
        );
    }

    #[test]
    fn sample_json_nulls_missing_components_not_zero() {
        let sample = Sample {
            frontend: "unit".into(),
            n: 1,
            workload: Workload::Active,
            elapsed_s: 0.0,
            phase: "seed".into(),
            ready: 0,
            active: 0,
            resident_bytes: None,
            peak_resident_bytes: None,
            rust_allocations: None,
            rust_allocated_bytes: None,
            rust_live_bytes: None,
            snapshot_inflight_bytes: None,
            snapshot_inflight_capacity: None,
            v8_used_bytes: None,
            v8_total_bytes: None,
            gpu_tracked_bytes: None,
        };
        let v = sample.to_json();
        let obj = v.as_object().expect("object");
        for key in [
            "resident_bytes",
            "peak_resident_bytes",
            "rust_allocations",
            "rust_allocated_bytes",
            "rust_live_bytes",
            "snapshot_inflight_bytes",
            "snapshot_inflight_capacity",
            "v8_used_bytes",
            "v8_total_bytes",
            "gpu_tracked_bytes",
        ] {
            assert!(obj.get(key).unwrap().is_null(), "{key} should be null");
        }
    }

    fn unit_config(n: usize, workload: Workload) -> Config {
        Config {
            n,
            workload,
            warmup: Duration::from_secs(1),
            observe: Duration::from_secs(1),
        }
    }

    #[test]
    fn prepare_idle_mints_n_names_vault_under_temp_no_card() {
        let _lock = env_lock();
        let _g = EnvGuard::clear(&["RS2B0T", "BOT_MEMORY_OUTPUT"]);
        let run = Run::prepare(unit_config(1, Workload::Idle), "unit").expect("idle prepare");
        assert_eq!(run.names.len(), 1);
        assert!(
            run.vault.starts_with(std::env::temp_dir()),
            "vault {:?} not under temp",
            run.vault
        );
        assert!(!run.has_script_card(), "idle must not load a script card");
        let idx = run.focus_index();
        assert!(
            idx < run.names.len(),
            "focus_index {idx} out of 0..{}",
            run.names.len()
        );
    }

    #[test]
    fn seed_runners_share_world_but_keep_independent_state() {
        let world = Arc::new(nav::world::NavWorld::from_parts(
            nav::collision::WorldCollision {
                origin: api::snapshot::WorldTile { x: 0, z: 0, level: 0 },
                width: 1, height: 1, walk: vec![0; 4], blocked: vec![0], flags: None,
            }, Default::default(), vec![],
        ));
        let mut a = seed_runner(seeded_idle_scenario(), "seed_a", Some(world.clone()));
        let b = seed_runner(seeded_idle_scenario(), "seed_b", Some(world.clone()));
        assert_eq!(Arc::strong_count(&world), 3, "both runners must retain the injected world");
        assert_eq!(a.runner.profile_name(), "seed_a");
        assert_eq!(b.runner.profile_name(), "seed_b");
        a.started = true;
        assert!(!b.started);
        drop(a);
        assert_eq!(Arc::strong_count(&world), 2);
        drop(b);
        assert_eq!(Arc::strong_count(&world), 1);
    }

    #[test]
    fn bind_seed_nav_from_play_preserves_arc_identity() {
        let _lock = env_lock();
        let _g = EnvGuard::clear(&["RS2B0T", "BOT_MEMORY_OUTPUT", "BOT_MEMORY_N", "BOT_MEMORY_WORKLOAD"]);
        let world = Arc::new(nav::world::NavWorld::from_parts(
            nav::collision::WorldCollision {
                origin: api::snapshot::WorldTile { x: 0, z: 0, level: 0 },
                width: 1, height: 1, walk: vec![0; 4], blocked: vec![0], flags: None,
            },
            Default::default(),
            vec![],
        ));
        let run = Run::prepare_unseeded(unit_config(2, Workload::SeededIdle), "unit")
            .expect("unseeded prepare");
        run.bind_seed_nav(SeedNav::FromPlay(Some(world.clone())))
            .expect("bind play world");
        let seeds = SEEDS.lock().unwrap();
        let map = seeds.as_ref().expect("seeds installed");
        assert_eq!(map.len(), 2);
        for name in &run.names {
            let seed = map.get(name).expect("named seed").lock().unwrap();
            let shared = seed.runner.shared_world().expect("seed must hold play world");
            assert!(
                Arc::ptr_eq(&world, &shared),
                "seed {name} must share Play Arc identity"
            );
        }
        // Play owner + 2 seed runners.
        assert!(Arc::strong_count(&world) >= 3);
    }

    #[test]
    fn bind_seed_nav_from_play_none_keeps_missing_pack() {
        let _lock = env_lock();
        let _g = EnvGuard::clear(&["RS2B0T", "BOT_MEMORY_OUTPUT"]);
        let run = Run::prepare_unseeded(unit_config(1, Workload::SeededIdle), "unit")
            .expect("unseeded prepare");
        // Play had no pack: FromPlay(None) must not attempt a second decode.
        run.bind_seed_nav(SeedNav::FromPlay(None))
            .expect("bind missing pack");
        let seeds = SEEDS.lock().unwrap();
        let seed = seeds
            .as_ref()
            .expect("seeds")
            .get(&run.names[0])
            .expect("seed")
            .lock()
            .unwrap();
        assert!(
            seed.runner.shared_world().is_none(),
            "missing Play pack must stay missing on seeds"
        );
    }

    #[test]
    fn prepare_with_seed_nav_from_play_matches_bind() {
        let _lock = env_lock();
        let _g = EnvGuard::clear(&["RS2B0T", "BOT_MEMORY_OUTPUT"]);
        let world = Arc::new(nav::world::NavWorld::from_parts(
            nav::collision::WorldCollision {
                origin: api::snapshot::WorldTile { x: 1, z: 2, level: 0 },
                width: 1, height: 1, walk: vec![0; 4], blocked: vec![0], flags: None,
            },
            Default::default(),
            vec![],
        ));
        let run = Run::prepare_with_seed_nav(
            unit_config(1, Workload::SeededIdle),
            "unit",
            SeedNav::FromPlay(Some(world.clone())),
        )
        .expect("prepare with play nav");
        let seeds = SEEDS.lock().unwrap();
        let shared = seeds
            .as_ref()
            .unwrap()
            .get(&run.names[0])
            .unwrap()
            .lock()
            .unwrap()
            .runner
            .shared_world()
            .expect("world");
        assert!(Arc::ptr_eq(&world, &shared));
    }

    #[test]
    fn seeded_idle_preserves_setup_and_ends_before_script_start() {
        let active = scenario::thiever_sustained_scenario();
        let idle = seeded_idle_scenario();
        let start = active.steps.iter().position(|s| matches!(s.kind, scenario::StepKind::StartScript)).unwrap();
        assert_eq!(idle.steps.len(), start);
        assert_eq!(idle.steps.iter().map(|s| s.name).collect::<Vec<_>>(), active.steps[..start].iter().map(|s| s.name).collect::<Vec<_>>());
        assert!(matches!(idle.steps.last().unwrap().kind, scenario::StepKind::DrainDialogs { .. }));
        assert!(matches!(idle.proof, scenario::Proof::ArrivedNear { x: 2661, z: 3306, level: 0, radius: 10 }));
        assert!(idle.settings.start_script.is_none());
    }

    #[test]
    fn prepare_seeded_idle_runs_seed_without_catalog() {
        let _lock = env_lock();
        let _g = EnvGuard::clear(&["RS2B0T", "BOT_MEMORY_OUTPUT", "BOT_MEMORY_N", "BOT_MEMORY_WORKLOAD"]);
        std::env::set_var("BOT_MEMORY_N", "1");
        std::env::set_var("BOT_MEMORY_WORKLOAD", "seeded-idle");
        let config = Config::from_env().expect("seeded idle config").unwrap();
        assert_eq!(config.workload.as_str(), "seeded-idle");
        let run = Run::prepare(config, "unit").expect("seed without catalog");
        assert!(!run.has_script_card());
        let seeds = SEEDS.lock().unwrap();
        assert_eq!(seeds.as_ref().unwrap().len(), 1);
        assert!(seeds.as_ref().unwrap().contains_key(&run.names[0]));
    }

    #[test]
    fn prepare_idle_n32_mints_thirty_two_unique_names() {
        let _lock = env_lock();
        let _g = EnvGuard::clear(&["RS2B0T", "BOT_MEMORY_OUTPUT"]);
        let run = Run::prepare(unit_config(32, Workload::Idle), "unit").expect("idle n=32");
        assert_eq!(run.names.len(), 32);
        let mut uniq = run.names.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), 32);
        assert!(!run.has_script_card());
        assert!(run.focus_index() < 32);
    }

    #[test]
    fn prepare_active_errors_without_rs2b0t() {
        let _lock = env_lock();
        let _g = EnvGuard::clear(&["RS2B0T", "BOT_MEMORY_OUTPUT"]);
        let err = match Run::prepare(unit_config(1, Workload::Active), "unit") {
            Err(e) => e,
            Ok(_) => panic!("expected RS2B0T error"),
        };
        assert!(err.contains("RS2B0T"), "expected RS2B0T error, got {err}");
    }

    #[test]
    fn prepare_lifecycle_errors_without_rs2b0t() {
        let _lock = env_lock();
        let _g = EnvGuard::clear(&["RS2B0T", "BOT_MEMORY_OUTPUT"]);
        let err = match Run::prepare(unit_config(1, Workload::Lifecycle), "unit") {
            Err(e) => e,
            Ok(_) => panic!("expected RS2B0T error"),
        };
        assert!(err.contains("RS2B0T"), "expected RS2B0T error, got {err}");
    }

    #[test]
    fn require_live_benchmark_errors_without_live_env() {
        let _lock = env_lock();
        let _g = EnvGuard::clear(&["LIVE"]);
        let err = require_live_benchmark().expect_err("LIVE unset");
        assert!(err.contains("LIVE=1"), "got {err}");
    }

    /// Minimal Run for failure-helper unit tests — no mint/vault/prepare.
    fn harness_stub(
        failure_capture: bool,
        diagnostics: bool,
        qualification_output: std::fs::File,
    ) -> Run {
        let sink = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/null")
            .expect("/dev/null");
        Run {
            config: unit_config(1, Workload::Idle),
            names: vec!["unit0".into()],
            vault: PathBuf::from("/tmp/274bot-fc-stub-vault-unused"),
            pass: String::new(),
            frontend: "unit",
            started: Instant::now(),
            warm: None,
            observing: None,
            last_sample: None,
            lifecycle_cycle: 0,
            stopped: false,
            teardown: None,
            card: None,
            output: sink.try_clone().expect("clone sink"),
            diagnostics,
            failure_capture,
            failure_boundary_attempted: false,
            single_renderer: false,
            render_policy: RenderPolicy::RotatingAll,
            diagnostic_output: diagnostics.then(|| sink.try_clone().expect("diag sink")),
            qualification_output,
        }
    }

    fn unique_qfile(tag: &str) -> (PathBuf, std::fs::File) {
        let dir = std::env::temp_dir().join(format!(
            "274bot-fc-stub-{}-{}",
            tag,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("qualification.jsonl");
        let f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        (path, f)
    }

    /// One prepare integration: env resolves failure_capture without diagnostics sidecar.
    #[test]
    fn prepare_failure_capture_without_diagnostics_sidecar() {
        let _lock = env_lock();
        let dir = std::env::temp_dir().join(format!(
            "274bot-fc-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("samples.jsonl");
        let _g = EnvGuard::clear(&[
            "RS2B0T",
            "BOT_MEMORY_OUTPUT",
            "BOT_MEMORY_DIAGNOSTICS",
            "BOT_MEMORY_FAILURE_CAPTURE",
        ]);
        std::env::set_var("BOT_MEMORY_OUTPUT", &out);
        std::env::set_var("BOT_MEMORY_FAILURE_CAPTURE", "1");
        std::env::remove_var("BOT_MEMORY_DIAGNOSTICS");
        let run = Run::prepare(unit_config(1, Workload::Idle), "unit").expect("prepare");
        assert!(run.failure_capture);
        assert!(!run.diagnostics);
        assert!(run.diagnostic_output.is_none());
        assert!(!run.failure_boundary_attempted);
        assert!(out.exists());
        assert!(out.with_extension("qualification.jsonl").exists());
        assert!(
            !out.with_extension("diagnostics.jsonl").exists(),
            "failure-capture alone must not create diagnostics.jsonl"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn handle_harness_failure_preserves_err_and_latches_on_writer_fail() {
        // Read-only fd forces best-effort writeln fail without mint/vault.
        let q = std::fs::OpenOptions::new()
            .read(true)
            .open("/dev/null")
            .expect("open /dev/null read-only");
        let mut run = harness_stub(true, false, q);
        let failure = String::from("Thiever seed/proof failed for unit0: boom");
        let err1 = run
            .handle_harness_failure(
                failure.clone(),
                |_| vec![],
                |_, _| panic!("diag must not run when diagnostics is off"),
            )
            .expect_err("must return harness failure");
        assert_eq!(err1, failure);
        assert!(run.failure_boundary_attempted);
        let err2 = run
            .handle_harness_failure(
                failure.clone(),
                |_| panic!("slots producer must not run when already latched"),
                |_, _| panic!("diag must not run when diagnostics is off"),
            )
            .expect_err("repeat must still return original failure");
        assert_eq!(err2, failure);
        assert!(run.failure_boundary_attempted);
    }

    #[test]
    fn handle_harness_failure_noop_when_flag_off() {
        let (qpath, q) = unique_qfile("off");
        let before = std::fs::metadata(&qpath).map(|m| m.len()).unwrap_or(0);
        let mut run = harness_stub(false, false, q);
        let err = run
            .handle_harness_failure(
                "unit0: script stopped".into(),
                |_| panic!("slots producer must not run when failure_capture is off"),
                |_, _| panic!("diag must not run when diagnostics is off"),
            )
            .expect_err("still returns failure");
        assert_eq!(err, "unit0: script stopped");
        assert!(!run.failure_boundary_attempted);
        let after = std::fs::metadata(&qpath).map(|m| m.len()).unwrap_or(0);
        assert_eq!(before, after, "flag off must not write a failure row");
        let _ = std::fs::remove_dir_all(qpath.parent().unwrap());
    }

    #[test]
    fn handle_harness_failure_diag_err_swallowed_when_capture_on() {
        let (_qpath, q) = unique_qfile("diag-cap");
        let mut run = harness_stub(true, true, q);
        let failure = String::from("orig-seed-err");
        let err = run
            .handle_harness_failure(
                failure.clone(),
                |_| vec![],
                |_, _| Err("diag-disk-full".into()),
            )
            .expect_err("must return failure");
        assert_eq!(
            err, failure,
            "failure_capture on: original Err survives diag write fail"
        );
        assert!(run.failure_boundary_attempted);
        let _ = std::fs::remove_dir_all(_qpath.parent().unwrap());
    }

    #[test]
    fn handle_harness_failure_diag_err_wins_when_capture_off() {
        let (_qpath, q) = unique_qfile("legacydiag");
        let mut run = harness_stub(false, true, q);
        let err = run
            .handle_harness_failure(
                "orig-seed-err".into(),
                |_| panic!("slots producer must not run when failure_capture is off"),
                |_, _| Err("diag-disk-full".into()),
            )
            .expect_err("legacy diag ? must surface");
        assert_eq!(err, "diag-disk-full");
        assert!(!run.failure_boundary_attempted);
        let _ = std::fs::remove_dir_all(_qpath.parent().unwrap());
    }

    #[test]
    fn failure_capture_and_diagnostics_flags_independent_on_stub() {
        let (_qpath, q) = unique_qfile("both");
        let run = harness_stub(true, true, q);
        assert!(run.failure_capture);
        assert!(run.diagnostics);
        assert!(run.diagnostic_output.is_some());
        let run_off = harness_stub(false, false, std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/null")
            .unwrap());
        assert!(!run_off.failure_capture);
        assert!(!run_off.diagnostics);
        let _ = std::fs::remove_dir_all(_qpath.parent().unwrap());
    }
}
