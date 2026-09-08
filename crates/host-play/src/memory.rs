//! Opt-in frontend memory benchmark harness.
//!
//! No effect unless `BOT_MEMORY_N` is set. Counting allocator is installed
//! only by frontend binaries — no logging or allocation inside allocator
//! callbacks. Sample fields are separate domains; never sum them and never
//! equate allocation counts with RSS.

use crate::{Play, PlayOptions};
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
/// Returns `(user_seconds, kernel/system_seconds)` when the OS sampler works.
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
    #[cfg(windows)]
    {
        // User + kernel from GetProcessTimes; optional fields stay null on fail.
        crate::rss::windows_cpu_user_kernel()
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
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

/// Live fields collected per fixture name at a qualification boundary.
/// Ordinal + slot ids are filled by [`qualification_slot_rows`] from `names`.
struct QualificationSlotFields {
    state: String,
    error: Option<String>,
    runtime: serde_json::Value,
    client: Option<serde_json::Value>,
    /// Live client scalar settings from SlotStatus; null until first observe.
    runtime_settings: Option<serde_json::Value>,
    /// Scoped render_profile row when profiler enabled and a match exists.
    renderer: Option<serde_json::Value>,
}

/// Serialize a captured client runtime settings snapshot (or null if never
/// observed). Additive on the slot row; does not invent defaults.
fn runtime_settings_value(
    snap: Option<crate::ClientRuntimeSettingsSnapshot>,
) -> Option<serde_json::Value> {
    snap.map(|s| {
        serde_json::json!({
            "lowmem": s.lowmem,
            "midi_active": s.midi_active,
            "midi_volume": s.midi_volume,
            "wave_enabled": s.wave_enabled,
            "wave_volume": s.wave_volume,
            "draw": s.draw,
            // Client main-loop counter (freshness), not server/player generation.
            "loop_cycle": s.loop_cycle,
            "loop_cycle_meaning": "client_mainloop_counter",
        })
    })
}

/// Scoped render_profile evidence for one fixture name. Never dumps histories.
/// Off / missing stays unavailable; backend string is observed residency only.
fn renderer_evidence_for(
    name: &str,
    observations: Option<&[host::render_profile::SlotObservation]>,
) -> Option<serde_json::Value> {
    let Some(obs) = observations else {
        return None;
    };
    let id = host::render_profile::slot_id_for(name);
    // Prefer non-ended live row; fall back to any matching generation.
    let row = obs
        .iter()
        .find(|o| o.slot_id == id && !o.ended)
        .or_else(|| obs.iter().find(|o| o.slot_id == id))?;
    Some(serde_json::json!({
        "available": true,
        "source": "host::render_profile::read",
        "slot_id": row.slot_id,
        "generation": row.generation,
        "renderer_present": row.renderer_present,
        // Absent → null; cpu/cpu_fallback/gpu are observed backend labels.
        "backend": row.backend.as_str(),
        "draw": row.draw,
        "full_rate": row.full_rate,
        "updated_ms": row.updated_ms,
        "ended": row.ended,
    }))
}

/// Build one qualification slot row per entry in `names` (authoritative fixture
/// ordinal = enumerate order). Status/hash-map order must never drive ordinals.
fn qualification_slot_rows(
    names: &[String],
    mut fields_for: impl FnMut(&str) -> QualificationSlotFields,
) -> Vec<serde_json::Value> {
    names
        .iter()
        .enumerate()
        .map(|(ordinal, name)| {
            let f = fields_for(name);
            let mut row = serde_json::json!({
                "ordinal": ordinal,
                "name": name,
                // Run-local FNV ids: map instrumentation onto fixture ordinal.
                // Do not compare these ids numerically across separate runs.
                "responsiveness_slot_id": host::responsiveness_profile::slot_id_for(name),
                "cadence_slot_id": host::cadence::slot_id_for(name),
                "state": f.state,
                "error": f.error,
                "runtime": f.runtime,
                "client": f.client,
                // Additive: null until first observed frame publishes settings.
                "runtime_settings": f.runtime_settings,
            });
            // Additive renderer evidence when profile on and a row matched.
            if let Some(renderer) = f.renderer {
                row["renderer"] = renderer;
            }
            row
        })
        .collect()
}

/// Canonicalize `cache_dir` when the path exists; otherwise keep the selected
/// string and note why canonicalization failed. Never hashes cache contents.
fn cache_dir_evidence(cache_dir: &str) -> serde_json::Value {
    let path = PathBuf::from(cache_dir);
    match path.canonicalize() {
        Ok(canon) => serde_json::json!({
            "cache_dir": cache_dir,
            "cache_dir_canonical": canon.display().to_string(),
            "cache_dir_canonical_available": true,
            "cache_content_hash": serde_json::Value::Null,
            "cache_content_hash_reason": "not_hashed_at_boundary; launcher/preflight may hash path independently",
        }),
        Err(e) => serde_json::json!({
            "cache_dir": cache_dir,
            "cache_dir_canonical": serde_json::Value::Null,
            "cache_dir_canonical_available": false,
            "cache_dir_canonical_reason": format!("canonicalize failed: {e}"),
            "cache_content_hash": serde_json::Value::Null,
            "cache_content_hash_reason": "not_hashed_at_boundary; launcher/preflight may hash path independently",
        }),
    }
}

/// Unavailable marker for live per-slot client state that is not exposed on
/// `Play`/`SlotStatus` without invasive instrumentation. Never invent defaults.
fn unavailable_client_state(reason: &str) -> serde_json::Value {
    serde_json::json!({
        "available": false,
        "reason": reason,
    })
}

/// Requested wall/harness options recorded at a successful qualification boundary.
/// Distinguishes Play creation options from unobserved live client state.
fn qualification_settings_value(
    options: &PlayOptions,
    frontend: &str,
    config: &Config,
    render_policy: RenderPolicy,
    single_renderer: bool,
    diagnostics: bool,
    failure_capture: bool,
) -> serde_json::Value {
    let env_flag = |key: &str| std::env::var(key).as_deref() == Ok("1");
    let mut settings = match cache_dir_evidence(&options.cache_dir) {
        serde_json::Value::Object(map) => map,
        other => panic!("cache_dir_evidence must be object, got {other}"),
    };
    settings.insert("host".into(), options.host.clone().into());
    settings.insert("port".into(), options.port.into());
    // PlayOptions.lowmem is the wall option selected at Play::new. Per-slot
    // live Client.config.lowmem is captured on SlotStatus.runtime_settings
    // (memory-profile) after the first observe frame.
    settings.insert("lowmem_requested".into(), options.lowmem.into());
    settings.insert("mainland".into(), options.mainland.into());
    settings.insert("frontend".into(), frontend.into());
    settings.insert("n".into(), config.n.into());
    settings.insert("workload".into(), config.workload.as_str().into());
    settings.insert(
        "render_policy_requested".into(),
        render_policy.as_str().into(),
    );
    settings.insert("single_renderer".into(), single_renderer.into());
    settings.insert("diagnostics".into(), diagnostics.into());
    settings.insert("failure_capture".into(), failure_capture.into());
    // Env labels as requested at process boundary time — not proof the
    // corresponding profiler was initialized. Actual enablement is separate.
    settings.insert(
        "env_flags_requested".into(),
        serde_json::json!({
            "BOT_SCHEDULING_PROFILE": env_flag("BOT_SCHEDULING_PROFILE"),
            "BOT_RENDER_PROFILE": env_flag("BOT_RENDER_PROFILE"),
            "BOT_GPU_COMPLETION_PROFILE": env_flag("BOT_GPU_COMPLETION_PROFILE"),
            "BOT_RESPONSIVENESS_PROFILE": env_flag("BOT_RESPONSIVENESS_PROFILE"),
            "BOT_RESPONSIVENESS_FINE": env_flag("BOT_RESPONSIVENESS_FINE"),
        }),
    );
    settings.insert(
        "scheduling_profile_enabled".into(),
        host::cadence::enabled().into(),
    );
    settings.insert(
        "render_profile_enabled".into(),
        host::render_profile::enabled().into(),
    );
    settings.insert(
        "gpu_completion_profile_enabled".into(),
        host::render_profile::gpu_completion_enabled().into(),
    );
    settings.insert(
        "responsiveness_profile_enabled".into(),
        host::responsiveness_profile::enabled().into(),
    );
    settings.insert(
        "responsiveness_fine_enabled".into(),
        host::responsiveness_profile::fine_enabled().into(),
    );
    // Per-slot live scalars are on slots[].runtime_settings (null until first
    // observe). Global markers point at that source — do not invent values.
    settings.insert(
        "client_lowmem_actual".into(),
        serde_json::json!({
            "available": true,
            "source": "per_slot",
            "path": "slots[].runtime_settings.lowmem",
            "note": "null until first observed frame; live Client.config.lowmem scalar, not PlayOptions.lowmem_requested",
        }),
    );
    settings.insert(
        "client_audio_actual".into(),
        serde_json::json!({
            "settings_available": true,
            "settings_source": "per_slot",
            "settings_path": "slots[].runtime_settings.{midi_active,midi_volume,wave_enabled,wave_volume}",
            "settings_note": "MIDI/wave enable+volume scalars from live Client; null until first observed frame",
            "physical_output_available": false,
            "physical_output_reason": "host speaker/device/sink ownership is not observed; do not infer physical audio output from midi/wave booleans",
        }),
    );
    settings.insert(
        "client_renderer_actual".into(),
        if host::render_profile::enabled() {
            serde_json::json!({
                "available": true,
                "source": "per_slot",
                "path": "slots[].renderer",
                "note": "scoped host::render_profile::read match by slot_id_for(name); absent when no matching row; requested GPU is not actual backend",
            })
        } else {
            unavailable_client_state(
                "render_profile disabled; per-slot actual backend not published (requested GPU is not actual backend)",
            )
        },
    );
    serde_json::Value::Object(settings)
}

/// Successful observe-start / observe-end qualification row. Slots first, then
/// elapsed_s, then optional settings (additive; legacy keys preserved).
fn serialize_qualification_boundary(
    phase: &str,
    slots: Vec<serde_json::Value>,
    elapsed_s: f64,
    settings: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut value = serde_json::json!({
        "phase": phase,
        "elapsed_s": elapsed_s,
        "slots": slots,
    });
    if let Some(settings) = settings {
        value["settings"] = settings;
    }
    value
}

type ScriptCard = (
    script::JsCard,
    serde_json::Map<String, serde_json::Value>,
    Vec<(String, String)>,
);

/// Additive fixed-window cohort journal publisher (process-local sidecar).
/// Active only when the existing responsiveness profile is already enabled at
/// observe-start — no new CLI/env flags.
struct CohortPublisher {
    boundaries: host::responsiveness_cohort::Boundaries,
    cursor: u64,
    /// Last published loss_count / journal_overflow_n so counter-only batches
    /// still emit when capacity prevented a receipt (empty vectors).
    last_loss_count: u64,
    last_journal_overflow_n: u64,
    sidecar: std::fs::File,
    sidecar_path: PathBuf,
    /// Harness `elapsed_s` of the observe-end qualification row (distinct from
    /// process-mono cohort boundaries).
    observe_end_elapsed_s: Option<f64>,
    terminal: Option<host::responsiveness_cohort::TerminalSummary>,
    /// True after slot/seed producers were stop_slot-joined and the barrier set.
    producers_joined: bool,
}

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
    /// Finite cohort drain after observe-end (cohort-on only). Scripts stay up.
    drain: Option<Instant>,
    teardown: Option<Instant>,
    card: Option<ScriptCard>,
    output: std::fs::File,
    /// Samples path used to derive the cohort sidecar name.
    output_path: PathBuf,
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
    cohort: Option<CohortPublisher>,
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
        // Opt-in BOT_NAV_CAPTURES: panel keeps GPU screenshot drain; TUI uses
        // data-only JSON drain. enable() is a no-op unless the env is set.
        crate::nav_capture::enable(&names);
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
            drain: None,
            teardown: None,
            card,
            output,
            output_path,
            diagnostics,
            failure_capture,
            failure_boundary_attempted: false,
            diagnostic_output,
            qualification_output,
            single_renderer,
            render_policy,
            cohort: None,
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
    pub fn poll(&mut self, play: &mut Play) -> Result<bool, String> {
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
        // Defer drain entry until after this tick's sample so the final observe
        // row keeps phase "observe" (operator: retain observe-end row before drain).
        // When the boundary fires, force a sample even if last_sample was <1s ago —
        // otherwise drain would flip with no final observe row on disk.
        let mut enter_drain_after_sample = false;
        if self.observing.is_none() && self.warm.is_some_and(|t| t.elapsed() >= self.config.warmup)
        {
            if !established {
                return Err("workload did not remain ready through warmup".into());
            }
            // Arm first so immutable START matches this observe-start boundary.
            self.arm_cohort_at_observe_start()?;
            self.write_qualification(play, "observe-start")?;
            // Prefer the Instant used for mono START when cohort-on so wall
            // observe duration tracks the same immutable window.
            self.observing = Some(self.cohort.as_ref().map(|_| Instant::now()).unwrap_or(now));
        }
        if let Some(observing) = self.observing {
            if self.config.workload == Workload::Lifecycle
                && self.teardown.is_none()
                && self.drain.is_none()
            {
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
            // Cohort-on: observe-end is the immutable mono END, not a drifted
            // Instant duration relative to a different poll clock.
            let observe_window_done = if let Some(c) = self.cohort.as_ref() {
                host::responsiveness_profile::mono_ns(Instant::now()) >= c.boundaries.end_mono_ns
            } else {
                observing.elapsed() >= self.config.observe
            };
            if observe_window_done && self.drain.is_none() && self.teardown.is_none() {
                // Stamp harness observe-end elapsed on the publisher BEFORE the
                // qualification write so cohort.observe_end_elapsed_s is not null
                // on the observe-end row (attach reads this field).
                if self.cohort.is_some() {
                    let elapsed = self.started.elapsed().as_secs_f64();
                    if let Some(c) = self.cohort.as_mut() {
                        c.observe_end_elapsed_s = Some(elapsed);
                    }
                }
                self.write_qualification(play, "observe-end")?;
                if self.cohort.is_some() {
                    // Keep ordinary runtime for the finite tail; scripts stay up
                    // through this sample, then drain begins after the write.
                    enter_drain_after_sample = true;
                } else {
                    for name in &self.names {
                        play.script_stop(name);
                    }
                    self.stopped = true;
                    self.teardown = Some(now);
                }
            }
        }
        // Cohort drain ends at absolute END+tail (process mono), never a
        // poll-relative extension of drain_at.elapsed().
        if self.drain.is_some() && self.teardown.is_none() {
            let tail_done = if let Some(c) = self.cohort.as_ref() {
                Self::cohort_tail_complete(
                    &c.boundaries,
                    host::responsiveness_profile::mono_ns(Instant::now()),
                )
            } else {
                false
            };
            if tail_done {
                for name in &self.names {
                    play.script_stop(name);
                }
                self.stopped = true;
                self.teardown = Some(now);
            }
        }

        // Force a sample on observe→drain boundary even when last_sample is
        // recent (<1s). Without this, END can flip drain with no final observe
        // row if the previous 1 Hz tick landed just before mono END.
        let sample_due = enter_drain_after_sample
            || self
                .last_sample
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(1));
        if sample_due
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
                phase: self.sample_phase_label().into(),
                ready,
                active,
                resident_bytes: crate::current_resident_bytes(),
                // sample_process first field is peak. Windows: never Some(0)
                // (fail/zero → None). Unix: preserve prior Some(sample.0)
                // including Some(0) when getrusage/sentinel yields 0.
                peak_resident_bytes: {
                    let peak = crate::sample_process().0;
                    #[cfg(windows)]
                    {
                        if peak == 0 {
                            None
                        } else {
                            Some(peak)
                        }
                    }
                    #[cfg(not(windows))]
                    {
                        Some(peak)
                    }
                },
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
            #[cfg(feature = "snapshot-dedup")]
            {
                // Bounded allocation-style diagnostics (not RSS). Play-local
                // directory only — no process-global username table walk.
                // One census sample for both slots and aggregate. Scratch peak
                // is unmeasured here: publish null + availability flag, not 0.
                let (slots, aggregate) = play.dedup_directory().census_sample(None);
                value["snapshot_dedup"] = serde_json::json!({
                    "scratch_peak_available": false,
                    "slots": slots,
                    "aggregate": aggregate,
                });
            }
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
            // Drain journal then attach refs *before* the durable sample write so
            // sidecar cursor/path land in the same JSONL row.
            self.durable_write_sample_line(&mut value)?;
            if self.diagnostics {
                self.write_diagnostics(play, None)?;
            }
            self.last_sample = Some(now);
        }

        if enter_drain_after_sample {
            self.drain = Some(Instant::now());
        }

        // Teardown wait remains 60s for logout/script settle (cohort-off and
        // cohort-on). Finite cohort tail is END+tail above — not this 60s.
        if self
            .teardown
            .is_some_and(|t| t.elapsed() >= Duration::from_secs(60))
        {
            // Before returning true (panel/TUI process::exit), prove producer
            // stop/join and emit terminal cohort summary when cohort-on.
            self.finish_cohort_shutdown(play)?;
            return Ok(true);
        }
        Ok(false)
    }

    // Two boundary reads preserve script progress evidence when verbose
    // diagnostic collection is disabled. Never drains logs or sends actions.
    // Slot reads run before elapsed_s so observe-boundary timing matches the
    // pre-failure-capture path (flag off / success rows unchanged).
    fn qualification_slots(&self, play: &Play) -> Vec<serde_json::Value> {
        let statuses = play.statuses();
        // One read per boundary when enabled; never entire histories.
        let render_obs = host::render_profile::read();
        let render_slice = render_obs.as_deref();
        qualification_slot_rows(&self.names, |name| {
            let status = statuses.iter().find(|s| s.username == name);
            let client = status.map(|s| {
                serde_json::json!({
                    "ingame": s.ingame,
                    "scene_state": s.scene_state,
                    "x": s.tile_x,
                    "z": s.tile_z,
                    "level": s.tile_level
                })
            });
            let runtime_settings =
                runtime_settings_value(status.and_then(|s| s.runtime_settings));
            let renderer = renderer_evidence_for(name, render_slice);
            QualificationSlotFields {
                state: format!("{:?}", play.script_state(name)),
                error: play.script_last_error(name),
                runtime: play.memory_script_progress(name),
                client,
                runtime_settings,
                renderer,
            }
        })
    }

    fn write_qualification(&mut self, play: &Play, phase: &str) -> Result<(), String> {
        // Slots first (progress evidence), then elapsed, then settings.
        let slots = self.qualification_slots(play);
        // Bracket the harness Instant read in wall time. The launcher has a
        // different elapsed-time origin and cannot supply this correlation.
        let wall_before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs_f64());
        let elapsed_s = self.started.elapsed().as_secs_f64();
        let wall_after = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs_f64());
        // Child module can read private parent `Play.options` — no public API.
        let settings = qualification_settings_value(
            &play.options,
            self.frontend,
            &self.config,
            self.render_policy,
            self.single_renderer,
            self.diagnostics,
            self.failure_capture,
        );
        let mut value = serialize_qualification_boundary(phase, slots, elapsed_s, Some(settings));
        value["elapsed_wall_bracket"] = serde_json::json!({
            "before_unix_s": wall_before,
            "after_unix_s": wall_after,
            "meaning": "SystemTime reads bracketing this row's harness elapsed Instant read",
        });
        self.attach_cohort_qualification_meta(&mut value, phase);
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

    /// Arm the fixed-window cohort only when the existing responsiveness
    /// profile is already enabled. No new flags. Uses race-safe `begin_armed`
    /// so OPT_IN is true before immutable START is sampled.
    fn arm_cohort_at_observe_start(&mut self) -> Result<(), String> {
        if !host::responsiveness_profile::enabled() {
            return Ok(());
        }
        if self.cohort.is_some() {
            return Ok(());
        }
        let observe_ns = u64::try_from(self.config.observe.as_nanos())
            .map_err(|_| "observe duration exceeds u64 nanos".to_string())?;
        if observe_ns == 0 {
            return Err("cohort observe duration must be non-zero".into());
        }
        let tail_ns = host::responsiveness_cohort::DEFAULT_TAIL_NS;
        let capacity = host::responsiveness_cohort::DEFAULT_CAPACITY;
        let boundaries = host::responsiveness_cohort::begin_armed(
            observe_ns,
            tail_ns,
            capacity,
            || host::responsiveness_profile::mono_ns(Instant::now()),
        )
        .map_err(|e| format!("cohort begin_armed failed: {e:?}"))?;
        let sidecar_path = self
            .output_path
            .with_extension("cohort.jsonl");
        let mut sidecar = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&sidecar_path)
            .map_err(|e| format!("cohort sidecar {}: {e}", sidecar_path.display()))?;
        let header = serde_json::json!({
            "record": "cohort-header",
            "schema_version": host::responsiveness_cohort::SCHEMA_VERSION,
            "tail_name": host::responsiveness_cohort::DEFAULT_TAIL_NAME,
            "tail_ns": tail_ns,
            "observe_ns": observe_ns,
            "capacity": capacity,
            "frontend": self.frontend,
            "input_population": if self.frontend == "panel" && self.render_policy.pins_focus() {
                serde_json::json!({"kind": "focused-one", "slots": [0]})
            } else if self.frontend == "tui" {
                serde_json::json!({"kind": "tui-endpoint", "note": "TUI flush endpoint; not panel texture present"})
            } else {
                serde_json::json!({"kind": "all-run-slots", "n": self.names.len()})
            },
            "decode_population": {"kind": "all-run-slots", "n": self.names.len()},
            "boundaries": {
                "start_mono_ns": boundaries.start_mono_ns,
                "end_mono_ns": boundaries.end_mono_ns,
                "tail_ns": boundaries.tail_ns,
            },
            "clock_domain": host::responsiveness_profile::CLOCK_DOMAIN,
            "sidecar_path": sidecar_path.display().to_string(),
        });
        writeln!(sidecar, "{header}").map_err(|e| e.to_string())?;
        sidecar.sync_all().map_err(|e| e.to_string())?;
        self.cohort = Some(CohortPublisher {
            boundaries,
            cursor: 0,
            last_loss_count: 0,
            last_journal_overflow_n: 0,
            sidecar,
            sidecar_path,
            observe_end_elapsed_s: None,
            terminal: None,
            producers_joined: false,
        });
        Ok(())
    }

    /// Production phase label used by sample rows (single source of truth).
    fn sample_phase_label(&self) -> &'static str {
        if self.teardown.is_some() {
            "teardown"
        } else if self.drain.is_some() {
            "drain"
        } else if self.observing.is_some() {
            "observe"
        } else if self.warm.is_some() {
            "warmup"
        } else {
            "seed"
        }
    }

    /// Production durable sample write: drain journal, attach cohort refs, then
    /// writeln. Callers must not attach refs after this write.
    fn durable_write_sample_line(&mut self, value: &mut serde_json::Value) -> Result<(), String> {
        self.drain_cohort_journal()?;
        self.attach_cohort_sample_refs(value);
        writeln!(self.output, "{value}").map_err(|e| e.to_string())
    }

    fn attach_cohort_qualification_meta(&self, value: &mut serde_json::Value, phase: &str) {
        let Some(c) = self.cohort.as_ref() else {
            return;
        };
        value["cohort"] = serde_json::json!({
            "schema_version": host::responsiveness_cohort::SCHEMA_VERSION,
            "present": true,
            "phase_tag": phase,
            "sidecar_path": c.sidecar_path.display().to_string(),
            "boundaries": {
                "start_mono_ns": c.boundaries.start_mono_ns,
                "end_mono_ns": c.boundaries.end_mono_ns,
                "tail_ns": c.boundaries.tail_ns,
            },
            "tail_name": host::responsiveness_cohort::DEFAULT_TAIL_NAME,
            "observe_end_elapsed_s": c.observe_end_elapsed_s,
            "qualification_elapsed_s_is_harness_not_cohort_mono": true,
        });
    }

    fn attach_cohort_sample_refs(&self, value: &mut serde_json::Value) {
        let Some(c) = self.cohort.as_ref() else {
            return;
        };
        value["cohort"] = serde_json::json!({
            "schema_version": host::responsiveness_cohort::SCHEMA_VERSION,
            "present": true,
            "sidecar_path": c.sidecar_path.display().to_string(),
            "cursor": c.cursor,
            "boundaries": {
                "start_mono_ns": c.boundaries.start_mono_ns,
                "end_mono_ns": c.boundaries.end_mono_ns,
                "tail_ns": c.boundaries.tail_ns,
            },
            "tail_name": host::responsiveness_cohort::DEFAULT_TAIL_NAME,
            "terminal": c.terminal.as_ref().map(|t| serde_json::json!({
                "available": t.available,
                "records_n": t.records_n,
                "losses_n": t.losses_n,
                "pending_n": t.pending_n,
            })),
        });
    }

    /// Absolute END+tail gate (process mono). Not Instant drain_at.elapsed().
    fn cohort_tail_complete(boundaries: &host::responsiveness_cohort::Boundaries, now_mono: u64) -> bool {
        now_mono >= boundaries.end_mono_ns.saturating_add(boundaries.tail_ns)
    }

    /// Bounded periodic journal drain (1 Hz sample cadence). Durable write;
    /// advances exact returned cursor. Fail-closed on extract/write errors.
    /// Emits counter-only batches when loss_count / journal_overflow_n move
    /// even if records and losses vectors are empty (capacity prevented receipt).
    fn drain_cohort_journal(&mut self) -> Result<(), String> {
        let Some(c) = self.cohort.as_mut() else {
            return Ok(());
        };
        if c.terminal.is_some() {
            return Ok(());
        }
        let batch = host::responsiveness_cohort::extract_since(c.cursor)
            .map_err(|e| format!("cohort extract_since({}): {e:?}", c.cursor))?;
        c.cursor = batch.next_cursor;
        let counters_moved = batch.loss_count != c.last_loss_count
            || batch.journal_overflow_n != c.last_journal_overflow_n;
        let should_emit = !batch.records.is_empty()
            || !batch.losses.is_empty()
            || batch.complete
            || counters_moved;
        if !should_emit {
            return Ok(());
        }
        c.last_loss_count = batch.loss_count;
        c.last_journal_overflow_n = batch.journal_overflow_n;
        let mut row = serde_json::json!({
            "record": "cohort-batch",
            "schema_version": batch.schema_version,
            "boundaries": batch.boundaries,
            "records": batch.records,
            "losses": batch.losses,
            "next_cursor": batch.next_cursor,
            "complete": batch.complete,
            "loss_count": batch.loss_count,
            "journal_overflow_n": batch.journal_overflow_n,
        });
        // Retain unavailable reason when capacity prevented a receipt even if
        // the journal vectors are empty for this drain tick.
        if batch.journal_overflow_n > 0 {
            row["unavailable_reason"] = serde_json::json!("journal_overflow");
            row["available"] = serde_json::json!(false);
        }
        writeln!(c.sidecar, "{row}").map_err(|e| format!("cohort sidecar write: {e}"))?;
        c.sidecar
            .sync_all()
            .map_err(|e| format!("cohort sidecar sync: {e}"))?;
        if batch.journal_overflow_n > 0 {
            return Err(format!(
                "cohort journal overflow detected: {}",
                batch.journal_overflow_n
            ));
        }
        Ok(())
    }

    /// Finalize journal after producers are proven closed. Used by
    /// `finish_cohort_shutdown` and by unit tests that set the barrier without
    /// a live Play.
    fn finish_cohort_journal_after_barrier(&mut self) -> Result<(), String> {
        let Some(c) = self.cohort.as_mut() else {
            return Ok(());
        };
        if c.terminal.is_some() {
            return Ok(());
        }
        let mut now_mono = host::responsiveness_profile::mono_ns(Instant::now());
        let need = c
            .boundaries
            .end_mono_ns
            .saturating_add(c.boundaries.tail_ns);
        if now_mono < need {
            return Err(format!(
                "cohort finalize before END+tail: now={now_mono} need={need}"
            ));
        }
        now_mono = host::responsiveness_profile::mono_ns(Instant::now()).max(need);
        let summary = host::responsiveness_cohort::finalize(now_mono)
            .map_err(|e| format!("cohort finalize: {e:?}"))?;

        let batch = host::responsiveness_cohort::extract_since(c.cursor)
            .map_err(|e| format!("cohort final extract_since({}): {e:?}", c.cursor))?;
        c.cursor = batch.next_cursor;
        c.last_loss_count = batch.loss_count;
        c.last_journal_overflow_n = batch.journal_overflow_n;
        let mut batch_row = serde_json::json!({
            "record": "cohort-batch",
            "schema_version": batch.schema_version,
            "boundaries": batch.boundaries,
            "records": batch.records,
            "losses": batch.losses,
            "next_cursor": batch.next_cursor,
            "complete": batch.complete,
            "loss_count": batch.loss_count,
            "journal_overflow_n": batch.journal_overflow_n,
        });
        if batch.journal_overflow_n > 0 {
            batch_row["unavailable_reason"] = serde_json::json!("journal_overflow");
            batch_row["available"] = serde_json::json!(false);
        }
        writeln!(c.sidecar, "{batch_row}").map_err(|e| e.to_string())?;

        let terminal_row = serde_json::json!({
            "record": "cohort-terminal",
            "schema_version": summary.schema_version,
            "boundaries": summary.boundaries,
            "terminal": summary.terminal,
            "available": summary.available,
            "records_n": summary.records_n,
            "losses_n": summary.losses_n,
            "pending_n": summary.pending_n,
            "tail_name": host::responsiveness_cohort::DEFAULT_TAIL_NAME,
            "producers_joined": c.producers_joined,
            "observe_end_elapsed_s": c.observe_end_elapsed_s,
            "frontend": self.frontend,
        });
        writeln!(c.sidecar, "{terminal_row}").map_err(|e| e.to_string())?;
        c.sidecar.sync_all().map_err(|e| e.to_string())?;
        c.terminal = Some(summary.clone());

        if !summary.available {
            return Err(format!(
                "cohort terminal unavailable: records_n={} losses_n={} pending_n={}",
                summary.records_n, summary.losses_n, summary.pending_n
            ));
        }
        if batch.journal_overflow_n > 0 {
            return Err(format!(
                "cohort terminal journal overflow: {}",
                batch.journal_overflow_n
            ));
        }
        Ok(())
    }

    /// Stop/join every publishing slot producer, set the producer barrier,
    /// finalize at absolute END+tail, drain remaining journal, write terminal
    /// summary. Must run before panel/TUI `process::exit` on poll(true).
    /// Does not treat script_stop or poll-true alone as producer closure.
    pub fn finish_cohort_shutdown(&mut self, play: &mut Play) -> Result<(), String> {
        if self.cohort.is_none() {
            return Ok(());
        }
        if self
            .cohort
            .as_ref()
            .is_some_and(|c| c.terminal.is_some())
        {
            return Ok(());
        }
        // Join all run slots that can still publish. stop_slot is the proven
        // path; Play::join alone waits forever on unstopped threads.
        let names: Vec<String> = self.names.clone();
        for name in &names {
            play.script_stop(name);
            play.stop_slot(name);
        }
        *SEEDS.lock().unwrap() = Some(HashMap::new());
        if let Some(c) = self.cohort.as_mut() {
            c.producers_joined = true;
        }

        host::responsiveness_cohort::acknowledge_producers_closed()
            .map_err(|e| format!("cohort acknowledge_producers_closed: {e:?}"))?;

        self.finish_cohort_journal_after_barrier()
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
    #[cfg(any(unix, windows))]
    fn process_cpu_time_is_available_and_monotonic() {
        let before = process_cpu_seconds().expect("process cpu sample");
        let after = process_cpu_seconds().expect("process cpu sample");
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

    // Keep these fixtures on the OS null device; the read-only handle below
    // must still reject writes on both platforms.
    #[cfg(windows)]
    const NULL_DEVICE: &str = "NUL";
    #[cfg(not(windows))]
    const NULL_DEVICE: &str = "/dev/null";

    /// Minimal Run for failure-helper unit tests — no mint/vault/prepare.
    fn harness_stub(
        failure_capture: bool,
        diagnostics: bool,
        qualification_output: std::fs::File,
    ) -> Run {
        let sink = std::fs::OpenOptions::new()
            .write(true)
            .open(NULL_DEVICE)
            .expect("open null device");
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
            drain: None,
            teardown: None,
            card: None,
            output: sink.try_clone().expect("clone sink"),
            output_path: PathBuf::from("/tmp/274bot-fc-stub-samples-unused.jsonl"),
            diagnostics,
            failure_capture,
            failure_boundary_attempted: false,
            single_renderer: false,
            render_policy: RenderPolicy::RotatingAll,
            diagnostic_output: diagnostics.then(|| sink.try_clone().expect("diag sink")),
            qualification_output,
            cohort: None,
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
            .open(NULL_DEVICE)
            .expect("open null device read-only");
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
            .open(NULL_DEVICE)
            .unwrap());
        assert!(!run_off.failure_capture);
        assert!(!run_off.diagnostics);
        let _ = std::fs::remove_dir_all(_qpath.parent().unwrap());
    }

    fn stub_fields(name: &str) -> QualificationSlotFields {
        QualificationSlotFields {
            state: format!("Running-{name}"),
            error: None,
            runtime: serde_json::json!({"dispatched": 1}),
            client: Some(serde_json::json!({
                "ingame": true,
                "scene_state": 2,
                "x": 1,
                "z": 2,
                "level": 0
            })),
            // Untouched: null until first observe (distinct from observed false).
            runtime_settings: None,
            renderer: None,
        }
    }

    fn assert_slot_legacy_and_ids(row: &serde_json::Value, ordinal: u64, name: &str) {
        assert_eq!(row["ordinal"], ordinal);
        assert_eq!(row["name"], name);
        assert_eq!(
            row["responsiveness_slot_id"],
            host::responsiveness_profile::slot_id_for(name)
        );
        assert_eq!(row["cadence_slot_id"], host::cadence::slot_id_for(name));
        assert_eq!(row["state"], format!("Running-{name}"));
        assert!(row["error"].is_null());
        assert_eq!(row["runtime"]["dispatched"], 1);
        assert_eq!(row["client"]["ingame"], true);
        // Legacy keys still present; runtime_settings additive (null until observe).
        for key in ["name", "state", "error", "runtime", "client", "runtime_settings"] {
            assert!(row.get(key).is_some(), "missing key {key}");
        }
        assert!(row["runtime_settings"].is_null());
        assert!(row.get("renderer").is_none());
    }

    #[test]
    fn qualification_slot_rows_stable_ordinals_for_n1_and_n16() {
        for n in [1usize, 16] {
            let names: Vec<String> = (0..n).map(|i| format!("fix{i}")).collect();
            let rows = qualification_slot_rows(&names, |name| stub_fields(name));
            assert_eq!(rows.len(), n, "N={n} must emit one row per name");
            let mut seen = std::collections::BTreeSet::new();
            for (i, row) in rows.iter().enumerate() {
                let name = &names[i];
                assert_slot_legacy_and_ids(row, i as u64, name);
                assert!(seen.insert(name.clone()), "duplicate name at N={n}");
                assert!(
                    seen.insert(format!("ord:{}", row["ordinal"])),
                    "duplicate ordinal at N={n}"
                );
            }
            assert_eq!(seen.len(), n * 2);
        }
    }

    #[test]
    fn qualification_slot_rows_ignore_status_hash_order() {
        // Producer only sees names; status map order cannot reorder ordinals.
        let names = vec!["z_last".into(), "a_first".into(), "m_mid".into()];
        let rows = qualification_slot_rows(&names, |name| stub_fields(name));
        assert_eq!(
            rows.iter().map(|r| r["name"].as_str().unwrap()).collect::<Vec<_>>(),
            vec!["z_last", "a_first", "m_mid"]
        );
        assert_eq!(
            rows.iter().map(|r| r["ordinal"].as_u64().unwrap()).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn serialize_qualification_boundary_preserves_key_order_contract() {
        let names = vec!["unit0".into()];
        let slots = qualification_slot_rows(&names, |name| stub_fields(name));
        let options = PlayOptions {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp/274bot-qual-missing-cache".into(),
            lowmem: true,
            mainland: false,
        };
        let settings = qualification_settings_value(
            &options,
            "tui",
            &unit_config(1, Workload::Active),
            RenderPolicy::RotatingAll,
            false,
            false,
            true,
        );
        let value = serialize_qualification_boundary(
            "observe-start",
            slots,
            12.5,
            Some(settings),
        );
        // Top-level successful boundary keys.
        assert_eq!(value["phase"], "observe-start");
        assert_eq!(value["elapsed_s"], 12.5);
        assert!(value["slots"].is_array());
        assert!(value.get("settings").is_some());
        // Legacy top-level keys unchanged; settings additive.
        for key in ["phase", "elapsed_s", "slots"] {
            assert!(value.get(key).is_some(), "missing top-level {key}");
        }
        let s = &value["settings"];
        assert_eq!(s["host"], "127.0.0.1");
        assert_eq!(s["port"], 43594);
        assert_eq!(s["cache_dir"], "/tmp/274bot-qual-missing-cache");
        assert_eq!(s["lowmem_requested"], true);
        assert_eq!(s["frontend"], "tui");
        assert_eq!(s["n"], 1);
        assert_eq!(s["workload"], "active");
        assert_eq!(s["render_policy_requested"], "rotating-all");
        assert_eq!(s["single_renderer"], false);
        assert_eq!(s["diagnostics"], false);
        assert_eq!(s["failure_capture"], true);
        assert_eq!(s["cache_content_hash"], serde_json::Value::Null);
        // Live scalars are per-slot; globals point at slots[].runtime_settings.
        assert_eq!(s["client_lowmem_actual"]["available"], true);
        assert_eq!(s["client_lowmem_actual"]["source"], "per_slot");
        assert_eq!(
            s["client_lowmem_actual"]["path"],
            "slots[].runtime_settings.lowmem"
        );
        assert_eq!(s["client_audio_actual"]["settings_available"], true);
        assert_eq!(s["client_audio_actual"]["settings_source"], "per_slot");
        assert_eq!(s["client_audio_actual"]["physical_output_available"], false);
        assert!(s["client_audio_actual"]["physical_output_reason"]
            .as_str()
            .unwrap()
            .contains("not observed"));
        // render_profile off in unit tests → still unavailable at global marker.
        assert_eq!(s["client_renderer_actual"]["available"], false);
        assert!(s["client_renderer_actual"]["reason"]
            .as_str()
            .unwrap()
            .contains("render_profile"));
    }

    #[test]
    fn qualification_settings_distinguishes_lowmem_requested_from_actual() {
        let options = PlayOptions {
            host: "example.test".into(),
            port: 1,
            cache_dir: "/no/such/cache/dir/for-qual".into(),
            lowmem: false,
            mainland: true,
        };
        let s = qualification_settings_value(
            &options,
            "panel",
            &unit_config(16, Workload::Idle),
            RenderPolicy::FocusedOne,
            true,
            true,
            false,
        );
        assert_eq!(s["lowmem_requested"], false);
        assert_eq!(s["mainland"], true);
        assert_eq!(s["n"], 16);
        assert_eq!(s["frontend"], "panel");
        assert_eq!(s["render_policy_requested"], "focused-one");
        assert_eq!(s["single_renderer"], true);
        // Actual live lowmem is not invented as a boolean here — per_slot path.
        assert_ne!(s["client_lowmem_actual"], false);
        assert_ne!(s["client_lowmem_actual"], true);
        assert_eq!(s["client_lowmem_actual"]["available"], true);
        assert_eq!(s["client_lowmem_actual"]["source"], "per_slot");
        assert!(s["cache_dir_canonical"].is_null());
        assert_eq!(s["cache_dir_canonical_available"], false);
    }

    #[test]
    fn cache_dir_evidence_canonicalizes_existing_path_without_hash() {
        let dir = std::env::temp_dir().join(format!(
            "274bot-qual-cache-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let v = cache_dir_evidence(dir.to_str().unwrap());
        assert_eq!(v["cache_dir"], dir.to_str().unwrap());
        assert_eq!(v["cache_dir_canonical_available"], true);
        assert!(v["cache_dir_canonical"].as_str().unwrap().len() > 0);
        assert!(v["cache_content_hash"].is_null());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn serialize_qualification_boundary_omits_settings_when_none() {
        let value = serialize_qualification_boundary("observe-end", vec![], 1.0, None);
        assert_eq!(value["phase"], "observe-end");
        assert!(value.get("settings").is_none());
    }

    #[test]
    fn runtime_settings_none_vs_observed_false_are_distinct() {
        assert!(runtime_settings_value(None).is_none());
        let observed_false = crate::ClientRuntimeSettingsSnapshot {
            lowmem: false,
            midi_active: false,
            midi_volume: 0,
            wave_enabled: false,
            wave_volume: 0,
            draw: false,
            loop_cycle: 0,
        };
        let v = runtime_settings_value(Some(observed_false)).expect("Some snapshot");
        assert_eq!(v["lowmem"], false);
        assert_eq!(v["midi_active"], false);
        assert_eq!(v["wave_enabled"], false);
        assert_eq!(v["draw"], false);
        assert_eq!(v["loop_cycle"], 0);
        assert_eq!(v["loop_cycle_meaning"], "client_mainloop_counter");
        // Not null object — observed false is an object with false booleans.
        assert!(v.is_object());
    }

    #[test]
    fn qualification_rows_match_settings_by_name_not_status_order() {
        // Status vector order differs from fixture names order.
        let names = vec!["z_last".into(), "a_first".into(), "m_mid".into()];
        let snap_z = crate::ClientRuntimeSettingsSnapshot {
            lowmem: true,
            midi_active: true,
            midi_volume: 1,
            wave_enabled: true,
            wave_volume: 2,
            draw: true,
            loop_cycle: 10,
        };
        let snap_a = crate::ClientRuntimeSettingsSnapshot {
            lowmem: false,
            midi_active: false,
            midi_volume: 3,
            wave_enabled: false,
            wave_volume: 4,
            draw: false,
            loop_cycle: 20,
        };
        // Only z and a observed; m stays None.
        let by_name = |name: &str| match name {
            "z_last" => Some(snap_z),
            "a_first" => Some(snap_a),
            _ => None,
        };
        let rows = qualification_slot_rows(&names, |name| {
            let mut f = stub_fields(name);
            f.runtime_settings = runtime_settings_value(by_name(name));
            f
        });
        assert_eq!(rows[0]["name"], "z_last");
        assert_eq!(rows[0]["ordinal"], 0);
        assert_eq!(rows[0]["runtime_settings"]["loop_cycle"], 10);
        assert_eq!(rows[0]["runtime_settings"]["lowmem"], true);
        assert_eq!(rows[1]["name"], "a_first");
        assert_eq!(rows[1]["ordinal"], 1);
        assert_eq!(rows[1]["runtime_settings"]["loop_cycle"], 20);
        assert_eq!(rows[1]["runtime_settings"]["lowmem"], false);
        assert_eq!(rows[2]["name"], "m_mid");
        assert_eq!(rows[2]["ordinal"], 2);
        assert!(rows[2]["runtime_settings"].is_null());
    }

    #[test]
    fn renderer_evidence_for_matches_slot_id_and_skips_missing() {
        assert!(renderer_evidence_for("nobody", None).is_none());
        assert!(renderer_evidence_for("nobody", Some(&[])).is_none());
        let id = host::render_profile::slot_id_for("alice");
        const B: usize = host::render_profile::INTERVAL_BOUNDS_MS.len() + 1;
        let obs = host::render_profile::SlotObservation {
            slot_id: id,
            generation: 3,
            renderer_present: true,
            backend: host::render_profile::BackendObs::Cpu,
            prefer_cpu: Some(true),
            ingame: true,
            scene_state: 2,
            draw: true,
            full_rate: false,
            client_loop_n: 0,
            paint_n: 0,
            skip_n: 0,
            stable_paint_n: 0,
            transition_paint_n: 0,
            stable_paint_intervals: 0,
            stable_paint_interval_ns: 0,
            stable_paint_interval_buckets: [0; B],
            transition_paint_intervals: 0,
            transition_paint_interval_ns: 0,
            transition_paint_interval_buckets: [0; B],
            attach_n: 0,
            detach_n: 0,
            backend_change_n: 0,
            updated_ms: 1234,
            ended: false,
            gpu_frame_n: 0,
            cpu_frame_n: 0,
            gpu_registered_n: 0,
            gpu_completed_n: 0,
            gpu_dropped_n: 0,
            gpu_lost_n: 0,
            gpu_pending_n: 0,
            gpu_oldest_pending_age_ms: 0,
            gpu_stable_completed_n: 0,
            gpu_transition_completed_n: 0,
            gpu_completion_latency_n: 0,
            gpu_completion_latency_ns: 0,
            gpu_completion_latency_buckets: [0; B],
            gpu_stable_completion_intervals: 0,
            gpu_stable_completion_interval_ns: 0,
            gpu_stable_completion_interval_buckets: [0; B],
            gpu_transition_completion_intervals: 0,
            gpu_transition_completion_interval_ns: 0,
            gpu_transition_completion_interval_buckets: [0; B],
        };
        let v = renderer_evidence_for("alice", Some(&[obs.clone()])).expect("matched");
        assert_eq!(v["available"], true);
        assert_eq!(v["slot_id"], id);
        assert_eq!(v["generation"], 3);
        assert_eq!(v["backend"], "cpu");
        assert_eq!(v["draw"], true);
        assert_eq!(v["full_rate"], false);
        assert_eq!(v["updated_ms"], 1234);
        assert_eq!(v["renderer_present"], true);
        // Wrong name → no match.
        assert!(renderer_evidence_for("bob", Some(&[obs])).is_none());
    }

    #[test]
    fn cohort_publisher_skips_when_responsiveness_profile_off() {
        // Profile is off by default in a fresh process; arm must be a no-op so
        // cohort-off harness behavior is preserved without new flags.
        // Subprocess isolation: profile enable is sticky process-global, and
        // RUST_TEST does NOT select the filter — pass explicit cargo/libtest
        // args with --exact.
        if std::env::var_os("HERMES_COHORT_OFF_CHILD").is_some() {
            assert!(
                !host::responsiveness_profile::enabled(),
                "fresh child must start with profile off"
            );
            host::responsiveness_cohort::harness_test_reset();
            let qdir = std::env::temp_dir().join(format!(
                "274bot-cohort-off-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&qdir).unwrap();
            let qpath = qdir.join("q.jsonl");
            let qfile = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&qpath)
                .unwrap();
            let mut run = harness_stub(false, false, qfile);
            run.output_path = qdir.join("samples.jsonl");
            run.arm_cohort_at_observe_start().expect("arm off path");
            assert!(run.cohort.is_none());
            let mut value = serde_json::json!({"phase": "observe"});
            run.attach_cohort_sample_refs(&mut value);
            assert!(value.get("cohort").is_none());
            run.attach_cohort_qualification_meta(&mut value, "observe-start");
            assert!(value.get("cohort").is_none());
            run.drain_cohort_journal().expect("drain off");
            run.finish_cohort_journal_after_barrier()
                .expect("finish off");
            let _ = std::fs::remove_dir_all(qdir);
            return;
        }
        let exe = std::env::current_exe().expect("test exe");
        let status = std::process::Command::new(&exe)
            .args([
                "memory::tests::cohort_publisher_skips_when_responsiveness_profile_off",
                "--exact",
                "--test-threads=1",
            ])
            .env("HERMES_COHORT_OFF_CHILD", "1")
            // Clear cargo/libtest inheritance that could expand the filter.
            .env_remove("RUST_TEST")
            .env_remove("CARGO_TARGET_TMPDIR")
            .status()
            .expect("spawn cohort-off child");
        assert!(status.success(), "cohort-off child failed: {status}");
    }

    #[test]
    fn cohort_tail_complete_uses_absolute_end_plus_tail_not_relative() {
        let b = host::responsiveness_cohort::Boundaries {
            start_mono_ns: 1_000,
            end_mono_ns: 11_000,
            tail_ns: 5_000,
        };
        // END+tail = 16_000. Instant-relative drain clocks must not matter.
        assert!(!Run::cohort_tail_complete(&b, 15_999));
        assert!(Run::cohort_tail_complete(&b, 16_000));
        assert!(Run::cohort_tail_complete(&b, 20_000));
        // Pre-END mono is never complete even if a drain Instant already elapsed.
        assert!(!Run::cohort_tail_complete(&b, 11_000));
    }

    #[test]
    fn cohort_publisher_production_lifecycle_cursor_drain_finalize() {
        // Production path: arm → producer start@START complete@tail →
        // durable_write (refs before writeln) → observe-end row before drain →
        // barrier → finalize. Uses real producer APIs + durable_write_sample_line.
        let _env = env_lock();
        host::responsiveness_cohort::harness_test_reset();
        host::responsiveness_profile::enable();

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("274bot-cohort-life-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let samples_path = dir.join("samples.jsonl");
        let samples = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&samples_path)
            .unwrap();
        let qpath = dir.join("q.jsonl");
        let qfile = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&qpath)
            .unwrap();

        let mut run = harness_stub(false, false, qfile);
        run.output = samples;
        run.output_path = samples_path.clone();
        // Short observe so finalize wait is tiny; tail stays DEFAULT_TAIL_NS —
        // tests complete the event inside a short synthetic tail by using
        // begin_armed with custom windows via arm (observe from config).
        run.config.observe = Duration::from_millis(5);

        run.arm_cohort_at_observe_start().expect("arm on");
        let c = run.cohort.as_ref().expect("cohort armed");
        let start = c.boundaries.start_mono_ns;
        let end = c.boundaries.end_mono_ns;
        let tail = c.boundaries.tail_ns;
        assert!(end > start);
        assert_eq!(tail, host::responsiveness_cohort::DEFAULT_TAIL_NS);
        let sidecar_path = c.sidecar_path.clone();

        // Included-start at START mono; complete inside the finite tail window.
        let id = host::responsiveness_cohort::start(
            1,
            1,
            start,
            host::responsiveness_cohort::Surface::Decode,
        )
        .expect("start at START");
        let complete_at = end + tail / 2;
        host::responsiveness_cohort::complete(id, complete_at);

        // Observing so sample_phase_label is observe before drain entry.
        run.observing = Some(Instant::now());
        assert_eq!(run.sample_phase_label(), "observe");

        let mut row = serde_json::json!({
            "phase": run.sample_phase_label(),
            "elapsed_s": 0.001,
        });
        // Production write path — must embed cohort cursor/path in the file.
        run.durable_write_sample_line(&mut row)
            .expect("durable observe sample");
        assert!(row.get("cohort").is_some(), "refs attached before write");
        assert_eq!(row["phase"], "observe");

        let sample_text = std::fs::read_to_string(&samples_path).unwrap();
        let sample_line = sample_text.lines().next().expect("sample line");
        let sample_json: serde_json::Value = serde_json::from_str(sample_line).unwrap();
        assert_eq!(sample_json["phase"], "observe");
        assert!(
            sample_json.get("cohort").is_some(),
            "serialized sample must include cohort refs (attach before writeln)"
        );
        assert!(
            sample_json["cohort"]["sidecar_path"]
                .as_str()
                .unwrap()
                .contains("cohort.jsonl"),
            "{sample_json}"
        );

        // Sidecar must have header + batch with the completed record.
        let side_text = std::fs::read_to_string(&sidecar_path).unwrap();
        let side_lines: Vec<&str> = side_text.lines().collect();
        assert!(
            side_lines.len() >= 2,
            "header + batch expected, got: {side_text}"
        );
        let header: serde_json::Value = serde_json::from_str(side_lines[0]).unwrap();
        assert_eq!(header["record"], "cohort-header");
        let batch: serde_json::Value = serde_json::from_str(side_lines[1]).unwrap();
        assert_eq!(batch["record"], "cohort-batch");
        assert!(
            batch["records"].as_array().map(|a| !a.is_empty()).unwrap_or(false),
            "completed start@START must appear in drained batch: {batch}"
        );
        let cursor_after = run.cohort.as_ref().unwrap().cursor;
        assert!(cursor_after > 0, "cursor must advance after drain");

        // Observe-end final row still phase observe; drain starts after.
        if let Some(c) = run.cohort.as_mut() {
            c.observe_end_elapsed_s = Some(0.005);
        }
        let mut observe_end_row = serde_json::json!({
            "phase": run.sample_phase_label(),
            "elapsed_s": 0.005,
            "boundary": "observe-end",
        });
        assert_eq!(observe_end_row["phase"], "observe");
        run.durable_write_sample_line(&mut observe_end_row)
            .expect("observe-end sample");
        run.drain = Some(Instant::now());
        assert_eq!(run.sample_phase_label(), "drain");
        let mut drain_row = serde_json::json!({
            "phase": run.sample_phase_label(),
            "elapsed_s": 0.006,
        });
        run.durable_write_sample_line(&mut drain_row)
            .expect("drain sample");
        assert_eq!(drain_row["phase"], "drain");

        // Wait absolute END+tail, barrier, finalize journal (no Play slots).
        let need = end.saturating_add(tail);
        loop {
            let now = host::responsiveness_profile::mono_ns(Instant::now());
            if now >= need {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(Run::cohort_tail_complete(
            &run.cohort.as_ref().unwrap().boundaries,
            host::responsiveness_profile::mono_ns(Instant::now())
        ));
        host::responsiveness_cohort::acknowledge_producers_closed()
            .expect("barrier after producers done");
        if let Some(c) = run.cohort.as_mut() {
            c.producers_joined = true;
        }
        run.finish_cohort_journal_after_barrier()
            .expect("finalize after END+tail + barrier");
        let term = run.cohort.as_ref().unwrap().terminal.as_ref().unwrap();
        assert!(term.available, "terminal must be available: {term:?}");
        assert!(
            term.records_n >= 1,
            "included completion must count: {term:?}"
        );

        let side_final = std::fs::read_to_string(&sidecar_path).unwrap();
        assert!(
            side_final.contains("\"record\":\"cohort-terminal\"")
                || side_final.contains("\"record\": \"cohort-terminal\""),
            "terminal row missing: {side_final}"
        );

        host::responsiveness_cohort::harness_test_reset();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn observe_end_qualification_meta_includes_elapsed_when_stamped_before_write() {
        // Production poll stamps observe_end_elapsed_s on CohortPublisher before
        // write_qualification("observe-end"). attach_cohort_qualification_meta
        // reads that field — stamp-after-write left cohort meta null.
        let _env = env_lock();
        host::responsiveness_cohort::harness_test_reset();
        host::responsiveness_profile::enable();

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("274bot-cohort-obs-end-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let qpath = dir.join("q.jsonl");
        let qfile = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&qpath)
            .unwrap();
        let samples_path = dir.join("samples.jsonl");
        let samples = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&samples_path)
            .unwrap();

        let mut run = harness_stub(false, false, qfile);
        run.output = samples;
        run.output_path = samples_path;
        run.config.observe = Duration::from_millis(5);
        run.arm_cohort_at_observe_start().expect("arm");

        // Mirror production order at observe-end boundary.
        let elapsed = run.started.elapsed().as_secs_f64();
        if let Some(c) = run.cohort.as_mut() {
            c.observe_end_elapsed_s = Some(elapsed);
        }
        let play = crate::run_channels(
            &crate::PlayOptions {
                host: "127.0.0.1".into(),
                port: 43594,
                cache_dir: dir.to_string_lossy().into(),
                lowmem: true,
                mainland: false,
            },
            vec![],
            0,
        );
        run.write_qualification(&play, "observe-end")
            .expect("observe-end qualification");

        let text = std::fs::read_to_string(&qpath).unwrap();
        let row: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(row["phase"], "observe-end");
        let meta = row.get("cohort").expect("cohort meta on observe-end row");
        assert!(
            meta["observe_end_elapsed_s"].as_f64().is_some(),
            "observe_end_elapsed_s must be non-null when stamped before write: {row}"
        );
        assert!(
            meta["observe_end_elapsed_s"].as_f64().unwrap() >= 0.0,
            "{row}"
        );

        // Without stamp, production attach would serialize null — prove the field
        // is read from publisher state (not invented at attach time).
        if let Some(c) = run.cohort.as_mut() {
            c.observe_end_elapsed_s = None;
        }
        let mut bare = serde_json::json!({"phase": "observe-end"});
        run.attach_cohort_qualification_meta(&mut bare, "observe-end");
        assert!(
            bare["cohort"]["observe_end_elapsed_s"].is_null(),
            "unstamped publisher must yield null meta: {bare}"
        );

        host::responsiveness_cohort::harness_test_reset();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn finish_cohort_shutdown_joins_slot_before_terminal() {
        // Production finish_cohort_shutdown must stop_slot/join publishers
        // before finalize — not script_stop alone.
        let _env = env_lock();
        host::responsiveness_cohort::harness_test_reset();
        host::responsiveness_profile::enable();

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("274bot-cohort-join-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let samples_path = dir.join("samples.jsonl");
        let samples = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&samples_path)
            .unwrap();
        let qfile = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join("q.jsonl"))
            .unwrap();

        let mut run = harness_stub(false, false, qfile);
        run.output = samples;
        run.output_path = samples_path;
        run.config.observe = Duration::from_millis(5);
        run.names = vec!["cohort_slot".into()];
        run.arm_cohort_at_observe_start().expect("arm");

        let start = run.cohort.as_ref().unwrap().boundaries.start_mono_ns;
        let end = run.cohort.as_ref().unwrap().boundaries.end_mono_ns;
        let tail = run.cohort.as_ref().unwrap().boundaries.tail_ns;
        let id = host::responsiveness_cohort::start(
            2,
            1,
            start,
            host::responsiveness_cohort::Surface::Decode,
        )
        .unwrap();
        host::responsiveness_cohort::complete(id, end + tail / 2);

        // Empty Play with a named stoppable slot so finish_cohort_shutdown
        // must stop_slot/join before terminal.
        let mut play = crate::run_channels(
            &crate::PlayOptions {
                host: "127.0.0.1".into(),
                port: 43594,
                cache_dir: dir.to_string_lossy().into(),
                lowmem: true,
                mainland: false,
            },
            vec![],
            0,
        );
        let stop = play.test_install_stoppable_slot("cohort_slot");
        assert!(play.slot_running("cohort_slot"));
        assert!(!stop.load(std::sync::atomic::Ordering::Relaxed));

        // Wait END+tail then finish (joins slot, barrier, terminal).
        let need = end.saturating_add(tail);
        while host::responsiveness_profile::mono_ns(Instant::now()) < need {
            std::thread::sleep(Duration::from_millis(1));
        }
        run.finish_cohort_shutdown(&mut play)
            .expect("finish joins + terminal");
        assert!(
            run.cohort.as_ref().unwrap().producers_joined,
            "producers_joined after stop_slot path"
        );
        assert!(
            run.cohort.as_ref().unwrap().terminal.is_some(),
            "terminal after join"
        );
        assert!(
            !play.slot_running("cohort_slot"),
            "slot handle must be joined/removed after stop_slot"
        );
        assert!(
            stop.load(std::sync::atomic::Ordering::Relaxed),
            "stop flag must be set by stop_slot before join returns"
        );

        host::responsiveness_cohort::harness_test_reset();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Production poll scheduling: if last_sample was <1s before mono END,
    /// poll must still write a final phase=observe row before entering drain.
    /// Manual durable_write fixtures cannot catch this gate.
    #[test]
    fn poll_forces_final_observe_sample_when_last_sample_recent_at_end() {
        let _env = env_lock();
        host::responsiveness_cohort::harness_test_reset();
        host::responsiveness_profile::enable();

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("274bot-cohort-poll-end-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let samples_path = dir.join("samples.jsonl");
        let samples = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&samples_path)
            .unwrap();
        let qpath = dir.join("q.jsonl");
        let qfile = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&qpath)
            .unwrap();

        let mut run = harness_stub(false, false, qfile);
        // Idle path: no seeds; empty play has ready=0 so stay past warm/observe
        // setup by pre-arming state (do not re-enter warmup arm).
        run.names.clear();
        run.config.n = 0;
        run.output = samples;
        run.output_path = samples_path.clone();
        run.config.observe = Duration::from_millis(5);
        run.warm = Some(Instant::now() - Duration::from_secs(10));
        run.observing = Some(Instant::now() - Duration::from_secs(1));
        // Recent sample would block the ordinary 1 Hz gate without force.
        run.last_sample = Some(Instant::now());

        run.arm_cohort_at_observe_start().expect("arm");
        let end = run.cohort.as_ref().unwrap().boundaries.end_mono_ns;
        // Spin until mono is past END so observe_window_done is true.
        while host::responsiveness_profile::mono_ns(Instant::now()) < end {
            std::thread::sleep(Duration::from_millis(1));
        }

        let before = std::fs::read_to_string(&samples_path).unwrap_or_default();
        let before_lines = before.lines().count();

        let mut play = crate::run_channels(
            &crate::PlayOptions {
                host: "127.0.0.1".into(),
                port: 43594,
                cache_dir: dir.to_string_lossy().into(),
                lowmem: true,
                mainland: false,
            },
            vec![],
            0,
        );
        run.poll(&mut play).expect("poll at END with recent last_sample");

        let after = std::fs::read_to_string(&samples_path).unwrap();
        let after_lines: Vec<&str> = after.lines().collect();
        assert!(
            after_lines.len() > before_lines,
            "poll must force a final observe sample despite recent last_sample; before={before_lines} after={}" ,
            after_lines.len()
        );
        let last: serde_json::Value =
            serde_json::from_str(after_lines.last().unwrap()).expect("sample json");
        assert_eq!(
            last["phase"], "observe",
            "forced boundary sample must still be phase observe before drain: {last}"
        );
        assert!(
            last.get("cohort").is_some(),
            "forced sample must carry cohort refs: {last}"
        );
        assert!(
            run.drain.is_some(),
            "drain must enter only after the forced final observe write"
        );
        assert_eq!(
            run.sample_phase_label(),
            "drain",
            "after forced write, phase is drain"
        );

        host::responsiveness_cohort::harness_test_reset();
        let _ = std::fs::remove_dir_all(dir);
    }
}
