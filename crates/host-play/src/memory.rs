//! Opt-in frontend memory benchmark harness.
//!
//! No effect unless `BOT_MEMORY_N` is set. Counting allocator is installed
//! only by frontend binaries — no logging or allocation inside allocator
//! callbacks. Sample fields are separate domains; never sum them and never
//! equate allocation counts with RSS.

use crate::Play;
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use api::interact::{cheat, tele_args};
use api::snapshot::WorldTile;

pub use crate::memory_startup::{
    mark_process_start, mark_startup, record_adapter, AdapterRecord, StartupMark,
};

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
static LIVE_BYTES: AtomicU64 = AtomicU64::new(0);

const PANEL_FRAME_BUCKETS: usize = 251;
static PANEL_FRAME_MS: [AtomicU64; PANEL_FRAME_BUCKETS] =
    [const { AtomicU64::new(0) }; PANEL_FRAME_BUCKETS];
static PANEL_FRAME_INTERVAL_MAX_NS: AtomicU64 = AtomicU64::new(0);

const READY_SETTLE: Duration = Duration::from_secs(2);

/// Whole-panel frame timer installed only by a memory-profile panel build.
///
/// Buckets are millisecond ceilings from 0 through 249; bucket 250 includes
/// every slower frame. The cumulative histogram lets the receipt runner
/// difference exactly the observation window it selected. The timer also
/// feeds the startup timeline: the space *between* render callbacks (where an
/// OS unresponsive interval lives) and the first presented frame.
pub struct PanelFrameTimer {
    start: Instant,
}

impl PanelFrameTimer {
    pub fn start() -> Self {
        let start = Instant::now();
        crate::memory_startup::render_started(start);
        Self { start }
    }

    /// The frame's swapchain image was just presented: call this right after
    /// `present()`, before readback mapping or other post-present work, so the
    /// first-presented milestone is not inflated by it. Frames that return
    /// early on a lost, outdated, timed-out or occluded surface never call
    /// this.
    pub fn presented(&self) {
        crate::memory_startup::mark_startup(StartupMark::FirstFramePresented);
    }
}

impl Drop for PanelFrameTimer {
    fn drop(&mut self) {
        let end = Instant::now();
        let elapsed = end.saturating_duration_since(self.start);
        let elapsed_us = elapsed.as_micros() as usize;
        let elapsed_ms = elapsed_us.div_ceil(1000).min(PANEL_FRAME_BUCKETS - 1);
        PANEL_FRAME_MS[elapsed_ms].fetch_add(1, Relaxed);
        let elapsed_ns = u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX);
        PANEL_FRAME_INTERVAL_MAX_NS.fetch_max(elapsed_ns, Relaxed);
        crate::memory_startup::render_ended(end);
    }
}

fn panel_frame_histogram() -> Vec<u64> {
    PANEL_FRAME_MS
        .iter()
        .map(|bucket| bucket.load(Relaxed))
        .collect()
}

fn take_panel_frame_interval_max_ns() -> u64 {
    PANEL_FRAME_INTERVAL_MAX_NS.swap(0, Relaxed)
}

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
    pub teardown: Duration,
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
            _ => {
                return Err(
                    "BOT_MEMORY_WORKLOAD must be idle, seeded-idle, active, or lifecycle".into(),
                )
            }
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
            teardown: duration("BOT_MEMORY_TEARDOWN_S", 60)?,
        }))
    }
}

pub fn parse_n(n: &str) -> Result<usize, String> {
    match n {
        "1" => Ok(1),
        "10" => Ok(10),
        "16" => Ok(16),
        "32" => Ok(32),
        "50" => Ok(50),
        "128" => Ok(128),
        _ => Err("BOT_MEMORY_N must be 1, 10, 16, 32, 50, or 128".into()),
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
    /// Existing panel `stress50`: one drawing head, 49 simulation-only slots.
    Stress50,
    /// Existing panel `stress50_full`: every wall member draws at full rate.
    Stress50Full,
}

impl RenderPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RotatingAll => "rotating-all",
            Self::FixedOne => "fixed-one",
            Self::FocusedOne => "focused-one",
            Self::FocusedPlusBackground => "focused-plus-background",
            Self::Stress50 => "stress50",
            Self::Stress50Full => "stress50-full",
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
    let policy = policy.as_deref().map(str::trim).filter(|s| !s.is_empty());
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
        (false, Some("stress50")) => Ok(RenderPolicy::Stress50),
        (false, Some("stress50-full")) => Ok(RenderPolicy::Stress50Full),
        (false, Some("rotating-all")) => Ok(RenderPolicy::RotatingAll),
        (true, Some(_)) => Err(
            "BOT_MEMORY_SINGLE_RENDERER conflicts with BOT_MEMORY_RENDER_POLICY".into(),
        ),
        (false, Some(other)) => Err(format!(
            "BOT_MEMORY_RENDER_POLICY must be rotating-all, fixed-one, focused-one, focused-plus-background, stress50, or stress50-full; got {other}"
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

/// Live bench gate: `LIVE=1` and a Local launch profile.
pub fn require_live_benchmark(class: crate::ProfileClass) -> Result<(), String> {
    if std::env::var("LIVE").as_deref() != Ok("1") || class != crate::ProfileClass::Local {
        return Err("memory benchmark requires LIVE=1 and a local profile".into());
    }
    Ok(())
}

struct Seed {
    runner: scenario::ScenarioRunner,
    started: bool,
    /// Scenario step names in order, so a stuck slot's record names the step
    /// it never left (the runner reports only an index).
    step_names: Vec<&'static str>,
    /// The step the runner was last polled executing: a runner that fails
    /// reports only its message, so this keeps where it stopped.
    last_step: Option<usize>,
    /// This slot's place in a `duel_arena` fleet's shared pair gate.
    duel: Option<DuelSlot>,
}

#[derive(Clone)]
struct DuelSlot {
    gate: Arc<DuelPairGate>,
    slot: usize,
}

/// Per-slot benchmark seed runners installed by [`Run::prepare`] for
/// seeded-idle, active, and lifecycle runs. Unseeded idle leaves this empty.
static SEEDS: Mutex<Option<HashMap<String, Arc<Mutex<Seed>>>>> = Mutex::new(None);

/// Called from the existing frontend slot observe hook. Drives the selected
/// benchmark's seed/proof; unseeded idle installs no seeds.
pub(crate) fn client_frame(c: &mut client::client::Client, name: &str, hold: bool) {
    mark_startup(StartupMark::FirstClientFrame);
    crate::memory_diagnostics::frame(c, name, hold);
    let seed = {
        let seeds = SEEDS.lock().unwrap();
        seeds.as_ref().and_then(|m| m.get(name).cloned())
    };
    let Some(seed) = seed else {
        return;
    };
    seed_frame(&mut seed.lock().unwrap(), c, hold);
}

/// One client frame of a slot's seed. A duel slot reports its tile to the
/// pair gate first, on every in-scene frame, including frames on which the
/// runner's scene-settle gate or a pending Start sends nothing.
fn seed_frame(seed: &mut Seed, c: &mut client::client::Client, hold: bool) {
    if let Some(duel) = &seed.duel {
        if let Some(tile) = duel_scene_tile(c) {
            duel.gate.observe(duel.slot, tile);
        }
    }
    if seed.runner.on_start_script() && !seed.started {
        return;
    }
    seed.runner.tick_with_hold(c, hold);
}

/// The local player's world tile, only while the scene is built (`scene_state
/// == 2`), the same base + route head a snapshot reports.
fn duel_scene_tile(c: &client::client::Client) -> Option<(i32, i32, i32)> {
    if !c.ingame || c.scene_state != 2 {
        return None;
    }
    c.local_player.as_ref().map(|lp| {
        (
            c.map_build_base_x + lp.route_x[0],
            c.map_build_base_z + lp.route_z[0],
            c.minusedlevel,
        )
    })
}

/// The installed duel fleet's zone-clearance record, for qualification rows.
fn duel_gate_summary() -> Option<serde_json::Value> {
    let seeds = SEEDS.lock().unwrap();
    let seed = seeds.as_ref()?.values().next()?;
    let duel = seed.lock().unwrap().duel.clone()?;
    Some(duel.gate.summary())
}

/// The same sustained Thiever setup, ending before any script is loaded.
/// Keep the original idle workload available as the unseeded login control.
fn seeded_idle_scenario() -> scenario::Scenario {
    let mut scenario = scenario::thiever_sustained_scenario();
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, scenario::StepKind::StartScript))
        .expect("Thiever StartScript");
    let arrival = scenario
        .steps
        .iter()
        .find(|step| step.name == "seed stats, food, and tele to the guard stand")
        .expect("Thiever seed")
        .wait
        .arm;
    scenario.steps.truncate(start);
    scenario.proof = arrival;
    scenario.settings.start_script = None;
    scenario.settings.terminal_shot = None;
    scenario
}

/// Preserve every scenario predicate while allowing a fleet to share the
/// local engine's actors and script work. Scenario budgets count dirty
/// snapshots, not wall time; the ordinary single-bot budgets are too short
/// when many benchmark slots contend for the same combat area.
fn widen_fleet_post_start_waits(scenario: &mut scenario::Scenario, n: usize) {
    if n <= 1 {
        return;
    }
    let minimum = (n as u32).saturating_mul(20).clamp(300, 1_800);
    let Some(start) = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, scenario::StepKind::StartScript))
    else {
        return;
    };
    for step in &mut scenario.steps[start + 1..] {
        step.wait.budget_ticks = step.wait.budget_ticks.max(minimum);
    }
}
/// Shared single-combat actors cannot award XP to every fleet member: the
/// frozen card counts another player's target disappearing as a kill. Keep the
/// ordinary N=1 XP proof, but require a fail-closed local engagement for W2
/// fleets so denied contenders do not block an otherwise representative load.
fn qualify_contentious_moss_fleet(
    scenario: &mut scenario::Scenario,
    n: usize,
) -> Result<(), String> {
    if n <= 1 || scenario.name != "moss_giant_bank_start" {
        return Ok(());
    }
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, scenario::StepKind::StartScript))
        .ok_or("moss fleet qualification is missing StartScript")?;
    let step = scenario.steps[start + 1..]
        .iter_mut()
        .find(|step| {
            step.name == "watch fresh Strength XP after the startup bank return"
                && step.wait.arm == scenario::Proof::FreshStatXpGain { id: 2, min: 1 }
        })
        .ok_or("moss fleet qualification is missing its post-return Strength XP watch")?;
    step.name = "watch the local player target a Moss giant after the startup bank return";
    step.wait.arm = scenario::Proof::LocalTargetingNpcName { name: "Moss giant" };
    Ok(())
}

/// Step budget for a fleet's completed-duel watch, admit wait, and park
/// Repeat. Staging admits one pair at a time, so the first pair's park-Repeat
/// waits until every later pair has fought and parked. A 3000-tick budget ended
/// that wait at ~21 min with 12 of 25 pairs parked; dirty snapshots run faster
/// than one per engine tick. The runner deadline is the real bound.
const DUEL_COMPLETED_DUEL_BUDGET_TICKS: u32 = 30_000;

/// Wall deadline for a serialized Duel Arena fleet. N=50 at ~90 s per staged
/// pair is longer than the ordinary 30-minute seed bound.
const DUEL_FLEET_DEADLINE: Duration = Duration::from_secs(3600);

/// Al Kharid bank: outside the frozen `DUEL_ZONE`, so an unstarted or
/// already-fought slot is not a `inDuelChallengeArea` candidate while another
/// pair Starts.
const DUEL_HOLD: WorldTile = WorldTile {
    x: 3269,
    z: 3167,
    level: 0,
};

/// Frozen `DUEL_CHALLENGE_ANCHOR` (3368, 3274): the lobby tile a pair Starts on.
const DUEL_LOBBY: WorldTile = WorldTile {
    x: 3368,
    z: 3274,
    level: 0,
};

/// Frozen `DuelArenaLogic.ts` `DUEL_ZONE` as `(min_x, max_x, min_z, max_z)`,
/// level 0: the arena and its challenge lobby.
const DUEL_ZONE: (i32, i32, i32, i32) = (3328, 3393, 3203, 3325);

/// Fewest steps from `(x, z)` onto any `DUEL_ZONE` tile (0 inside it). A step
/// moves at most one tile on each axis, so this is the Chebyshev distance.
/// Level is ignored: that only ever makes the fence stricter.
const fn duel_zone_steps(x: i32, z: i32) -> i32 {
    let dx = if x < DUEL_ZONE.0 {
        DUEL_ZONE.0 - x
    } else if x > DUEL_ZONE.1 {
        x - DUEL_ZONE.1
    } else {
        0
    };
    let dz = if z < DUEL_ZONE.2 {
        DUEL_ZONE.2 - z
    } else if z > DUEL_ZONE.3 {
        z - DUEL_ZONE.3
    } else {
        0
    };
    if dx > dz {
        dx
    } else {
        dz
    }
}

/// 59: the hold is this many steps from the nearest challenge-area tile.
const DUEL_HOLD_TO_ZONE_STEPS: i32 = duel_zone_steps(DUEL_HOLD.x, DUEL_HOLD.z);

/// The zone clearance: the fewest steps from `DUEL_ZONE` at which a waiting
/// or parked slot may be seen while the fence is up. The fence is up from a
/// slot's first hold until staging is over: the last pair has parked and
/// every slot has since reported a clear position (see [`DuelFence`]).
///
/// The frozen card walks a parked slot back toward the arena (`TravelToArena`).
/// The keep-parked Repeat re-teles it once it is 8 tiles from the hold, but
/// that send is unacknowledged and waits for the scenario's 2 s scene-settle
/// gate, so it cannot by itself keep an already-fought slot out of the lobby.
/// The fence does: [`DuelPairGate::observe`] runs on every in-scene client
/// frame of the slot (not settle-gated) and, under the admission lock,
/// records a named breach the first time a fenced slot is seen closer to the
/// zone than this. A breach refuses every further permit and fails the run.
///
/// Margin: a slot seen at the clearance is still 20 steps from the zone. A
/// running player takes at most two steps per 600 ms game tick, so it is at
/// least 10 game ticks (6 s) from being a `challengeTargets()` candidate —
/// against a slot frame that sees each `PLAYER_INFO` move and a benchmark poll
/// that fails the run on its next call. The fence bounds distance to the zone,
/// not to the hold: a slot that lands far away (live N=50 runs saw parked slots
/// land at the Lumbridge spawn, 106 steps out) is no candidate and is re-teled
/// by the Repeat. Nominal corrections leave room: the hold is 59 steps out and
/// the live N=50 correction excursion reached 22 tiles from the hold; the
/// qualification records carry the closest approach actually seen.
const DUEL_ZONE_CLEARANCE_STEPS: i32 = 20;

/// At least ten game ticks of running (two steps per tick) inside the fence.
const _: () = assert!(DUEL_ZONE_CLEARANCE_STEPS >= 2 * 10);
/// At least 30 steps of nominal correction excursion between hold and fence.
const _: () = assert!(DUEL_HOLD_TO_ZONE_STEPS - DUEL_ZONE_CLEARANCE_STEPS >= 30);

/// Why a duel fleet failed: the first fenced slot seen inside the zone
/// clearance, or a slot whose client thread stopped reporting while fenced.
#[derive(Clone, Debug, PartialEq, Eq)]
struct DuelFleetFailure {
    slot: usize,
    message: String,
}

/// Outcome of [`DuelPairGate::poll_start`] for a slot on its Start step.
#[derive(Clone, Debug, PartialEq, Eq)]
enum StartPermit {
    /// Start now: every slot outside this pair reported a clear position
    /// after this request, under the same lock.
    Granted,
    /// Not yet: a slot has not reported since the request, or an earlier
    /// request is ahead in the queue.
    Wait,
    /// Never: the fleet has failed.
    Refused(DuelFleetFailure),
}

/// The zone-clearance fence's lifecycle. It never lapses while a Start or a
/// challenge under exclusion can still happen: it comes down only after the
/// last pair has parked back at the hold and every slot has reported a clear
/// position since. A slot that entered the zone earlier is still there on
/// that report (only its own thread's keep-at-hold tele moves it out, and
/// that thread reports the in-zone tile first on the same frame), so a late
/// reporter breaches instead of being released.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DuelFence {
    Up,
    /// The last pair parked at this epoch.
    Lowering {
        since: u64,
    },
    /// Staging is over; parked slots are released to the lobby.
    Down,
}

/// One slot's standing with the admission owner.
#[derive(Default)]
struct DuelSlotState {
    holding: bool,
    parked: bool,
    /// Latest in-scene position report: `(epoch, steps to DUEL_ZONE)`.
    report: Option<(u64, i32)>,
}

/// One queued Start request: the slot and the epoch it was made at.
struct DuelStartRequest {
    slot: usize,
    epoch: u64,
}

/// Everything [`DuelPairGate`] decides on, behind its one lock. `epoch` is
/// the lock-order clock: every position report and every Start request takes
/// the next value, so "reported after the request" is a comparison of two
/// epochs taken under the same lock.
struct DuelAdmission {
    epoch: u64,
    slots: Vec<DuelSlotState>,
    admitted_pair: usize,
    /// Start requests in arrival order (owner token = slot); only the head
    /// may be granted.
    starts: VecDeque<DuelStartRequest>,
    fence: DuelFence,
    failure: Option<DuelFleetFailure>,
    granted: usize,
    /// Fewest zone steps at which a fenced slot was seen.
    closest_fenced_zone_steps: Option<i32>,
    /// Largest hold distance at which a fenced slot was seen.
    farthest_fenced_hold_distance: i32,
}

impl DuelAdmission {
    fn next_epoch(&mut self) -> u64 {
        self.epoch += 1;
        self.epoch
    }

    fn all_holding(&self) -> bool {
        self.slots.iter().all(|slot| slot.holding)
    }

    /// A slot must stay outside the zone clearance once it has reached the
    /// hold, unless it is the admitted pair on its way to or from its duel,
    /// until the fence is down. A parked member of the admitted pair is
    /// fenced too, and so is every slot once the last pair has parked.
    fn fenced(&self, slot: usize) -> bool {
        let state = &self.slots[slot];
        state.holding
            && self.fence != DuelFence::Down
            && (slot / 2 != self.admitted_pair || state.parked)
    }

    /// `slot`'s latest report is clear of the zone and newer than `epoch`.
    fn clear_since(&self, slot: usize, epoch: u64) -> bool {
        self.slots[slot]
            .report
            .is_some_and(|(at, steps)| at > epoch && steps >= DUEL_ZONE_CLEARANCE_STEPS)
    }

    /// Record the fleet's first failure; later ones keep the first.
    fn fail(&mut self, slot: usize, message: String) -> DuelFleetFailure {
        self.failure
            .get_or_insert(DuelFleetFailure { slot, message })
            .clone()
    }

    fn lower_once_reported(&mut self) {
        if let DuelFence::Lowering { since } = self.fence {
            if self.failure.is_none()
                && (0..self.slots.len()).all(|slot| self.clear_since(slot, since))
            {
                self.fence = DuelFence::Down;
            }
        }
    }

    /// Slots outside the head request's pair that have not reported clear
    /// since it was made. For the summary record only; the grant uses
    /// [`Self::has_unreported`], which allocates nothing under the lock.
    fn unreported_for(&self, request: &DuelStartRequest) -> Vec<usize> {
        (0..self.slots.len())
            .filter(|&other| self.unreported(other, request))
            .collect()
    }

    fn has_unreported(&self, request: &DuelStartRequest) -> bool {
        (0..self.slots.len()).any(|other| self.unreported(other, request))
    }

    fn unreported(&self, other: usize, request: &DuelStartRequest) -> bool {
        other / 2 != request.slot / 2 && !self.clear_since(other, request.epoch)
    }
}

/// Shared pair-admission owner for one duel fleet, the login FIFO's shape
/// (one mutex, owner tokens, a FIFO of requests, a grant, a retirement guard)
/// applied to Starts. Mint order is pairing: slots `(2i, 2i+1)` are pair `i`.
/// The frozen card challenges every named, out-of-combat player in the lobby
/// (`DuelArena.ts` `challengeTargets` / `sortChallengeTargets` / 1.5 s
/// `CHALLENGE_RESULT_WAIT_MS`); starting every slot at once is an all-to-all
/// race that can drop `opponent` to null inside a fight pen.
///
/// The owner keeps each Start's only free lobby candidate the partner. Every
/// slot thread's position report ([`Self::observe`]), every admission and
/// Start decision ([`Self::admitted`], [`Self::poll_start`]), every breach
/// and every fence transition happen under the one lock, so no decision reads
/// a half-updated fleet. A pair is admitted to leave the hold only after both
/// members of the previous pair have parked back at it (so a completed-duel
/// lobby snapshot cannot overlap a starting pair, including on the frame
/// `ScenarioRunner` only `advance_step()`s and the parking send has not yet
/// run). `Run::poll` Starts a slot only on a granted permit, and the grant
/// needs every slot outside the pair to have reported a clear position with
/// an epoch newer than the request: a latest reading from before the request
/// (a stalled or delayed slot thread) cannot admit it. A fenced slot seen
/// inside the clearance is a breach; after it no permit is granted and the
/// run fails with the breach.
///
/// Residual: a grant proves every other slot was clear at a moment after the
/// request, not at the instant the started card evaluates
/// `challengeTargets()`. A slot that walks in needs 10 game ticks from its
/// last clear report and is reported on the way. A fought slot moved
/// straight into `DUEL_ZONE` after a grant and before its next report would
/// be a candidate first and a breach (the run fails) second. The fixture has
/// no such mover: its corrective teles target the hold, observed respawns
/// land 106+ steps out, and the card walks at least 10 ticks through the
/// fence.
struct DuelPairGate {
    n: usize,
    state: parking_lot::Mutex<DuelAdmission>,
}

impl DuelPairGate {
    fn new(n: usize) -> Arc<Self> {
        Arc::new(Self {
            n,
            state: parking_lot::Mutex::new(DuelAdmission {
                epoch: 0,
                slots: (0..n).map(|_| DuelSlotState::default()).collect(),
                admitted_pair: 0,
                starts: VecDeque::new(),
                fence: DuelFence::Up,
                failure: None,
                granted: 0,
                closest_fenced_zone_steps: None,
                farthest_fenced_hold_distance: 0,
            }),
        })
    }

    fn mark_holding(&self, slot: usize) {
        self.state.lock().slots[slot].holding = true;
    }

    /// `slot`'s pair may leave the hold for the lobby: every slot has reached
    /// the hold, the pair is admitted, staging is not over, and the fleet has
    /// not failed. Its Start still needs a permit from [`Self::poll_start`].
    fn admitted(&self, slot: usize) -> bool {
        let state = self.state.lock();
        state.failure.is_none()
            && state.fence == DuelFence::Up
            && state.admitted_pair == slot / 2
            && state.all_holding()
    }

    /// Enter `slot`'s Start request once (its epoch is taken now) and grant
    /// it when it is the queue head and every slot outside its pair has
    /// reported clear of the zone since.
    fn poll_start(&self, slot: usize) -> StartPermit {
        let mut state = self.state.lock();
        if let Some(failure) = &state.failure {
            return StartPermit::Refused(failure.clone());
        }
        let pair = slot / 2;
        if state.fence != DuelFence::Up || state.admitted_pair != pair || !state.all_holding() {
            let message = format!(
                "duel fleet: refusing to Start slot {slot}: its pair {pair} is not the admitted \
                 pair (admitted pair {}, fence {:?}), so no further pair may Start",
                state.admitted_pair, state.fence
            );
            return StartPermit::Refused(state.fail(slot, message));
        }
        if !state.starts.iter().any(|request| request.slot == slot) {
            let epoch = state.next_epoch();
            state.starts.push_back(DuelStartRequest { slot, epoch });
        }
        let head = state.starts.front().expect("a request was just queued");
        if head.slot != slot || state.has_unreported(head) {
            return StartPermit::Wait;
        }
        state.starts.pop_front();
        state.granted += 1;
        StartPermit::Granted
    }

    /// `slot` is at the hold after its duel. Both members parked admits the
    /// next pair; the last pair parked starts lowering the fence.
    fn mark_parked(&self, slot: usize) {
        let mut state = self.state.lock();
        if std::mem::replace(&mut state.slots[slot].parked, true) {
            return;
        }
        let pair = slot / 2;
        if state.admitted_pair == pair
            && state.slots[2 * pair].parked
            && state.slots[2 * pair + 1].parked
        {
            state.admitted_pair += 1;
            if 2 * state.admitted_pair == self.n {
                let since = state.next_epoch();
                state.fence = DuelFence::Lowering { since };
            }
        }
    }

    /// Staging is over: parked slots may go to the lobby for observation.
    fn released(&self) -> bool {
        self.state.lock().fence == DuelFence::Down
    }

    /// One in-scene observation of `slot`'s own tile, from its own thread.
    fn observe(&self, slot: usize, (x, z, level): (i32, i32, i32)) {
        let mut state = self.state.lock();
        let epoch = state.next_epoch();
        let steps = duel_zone_steps(x, z);
        state.slots[slot].report = Some((epoch, steps));
        if state.fenced(slot) {
            state.closest_fenced_zone_steps = Some(
                state
                    .closest_fenced_zone_steps
                    .map_or(steps, |closest| closest.min(steps)),
            );
            let from_hold = (x - DUEL_HOLD.x).abs().max((z - DUEL_HOLD.z).abs());
            state.farthest_fenced_hold_distance =
                state.farthest_fenced_hold_distance.max(from_hold);
            if steps < DUEL_ZONE_CLEARANCE_STEPS {
                let stage = if 2 * state.admitted_pair < self.n {
                    format!("while pair {} was admitted", state.admitted_pair)
                } else {
                    "after the last pair parked, before the fence came down".to_string()
                };
                state.fail(
                    slot,
                    format!(
                        "duel fleet clearance breach: slot {slot} was seen at ({x}, {z}, level \
                         {level}), {steps} steps from the challenge area (clearance \
                         {DUEL_ZONE_CLEARANCE_STEPS}), {stage}; its keep-at-hold tele did not \
                         hold it, so no further pair may Start"
                    ),
                );
            }
        }
        state.lower_once_reported();
    }

    /// `slot`'s client thread has exited: it can no longer report, so while
    /// the fence is up the fleet fails instead of waiting on it.
    fn retire(&self, slot: usize) {
        let mut state = self.state.lock();
        state.starts.retain(|request| request.slot != slot);
        if state.fence != DuelFence::Down {
            state.fail(
                slot,
                format!(
                    "duel fleet: slot {slot}'s client thread exited while the zone-clearance \
                     fence was up, so its position can no longer be reported; no further pair \
                     may Start"
                ),
            );
        }
    }

    fn failure(&self) -> Option<DuelFleetFailure> {
        self.state.lock().failure.clone()
    }

    fn summary(&self) -> serde_json::Value {
        let state = self.state.lock();
        let pending_start = state.starts.front().map(|request| {
            serde_json::json!({
                "slot": request.slot,
                "awaiting_clear_reports_from": state.unreported_for(request),
            })
        });
        serde_json::json!({
            "zone_clearance_steps": DUEL_ZONE_CLEARANCE_STEPS,
            "hold_to_zone_steps": DUEL_HOLD_TO_ZONE_STEPS,
            "closest_fenced_zone_steps": state.closest_fenced_zone_steps,
            "farthest_fenced_hold_distance": state.farthest_fenced_hold_distance,
            "fence": match state.fence {
                DuelFence::Up => "up",
                DuelFence::Lowering { .. } => "lowering",
                DuelFence::Down => "down",
            },
            "start_permits_granted": state.granted,
            "pending_start": pending_start,
            "failure": state.failure.as_ref().map(|failure| failure.message.as_str()),
        })
    }
}

/// Slot-worker guard: every return and unwind of a slot's client thread
/// retires its place with the duel admission owner, so a slot that can no
/// longer report its position fails the fleet instead of stalling a permit.
pub(crate) struct DuelReportRetirement<'a> {
    pub(crate) name: &'a str,
}

impl Drop for DuelReportRetirement<'_> {
    fn drop(&mut self) {
        let seed = SEEDS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(|seeds| seeds.get(self.name).cloned());
        let Some(seed) = seed else {
            return;
        };
        let duel = seed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .duel
            .clone();
        if let Some(duel) = duel {
            duel.gate.retire(duel.slot);
        }
    }
}

fn duel_near(snap: &api::snapshot::GameSnapshot, dest: WorldTile, radius: i32) -> bool {
    snap.tile().is_some_and(|(x, z, level)| {
        level == dest.level && (x - dest.x).abs().max((z - dest.z).abs()) <= radius
    })
}

fn duel_tele_step(name: &'static str, dest: WorldTile, budget_ticks: u32) -> scenario::Step {
    scenario::Step {
        name,
        kind: scenario::StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &tele_args(dest.level, dest.x, dest.z));
                true
            }),
        },
        wait: scenario::Wait {
            arm: scenario::Proof::ArrivedNear {
                x: dest.x,
                z: dest.z,
                level: dest.level,
                radius: 8,
            },
            budget_ticks,
        },
    }
}

/// The Duel Arena card duels other players, so a fleet of it pairs among
/// itself: `n` slots make `n / 2` fights, and an odd fleet would leave one
/// slot without an opponent to qualify against. Each slot must finish a duel
/// after Start, seen from its own snapshots as a fight-pen visit followed by a
/// return to the lobby. A duel changes only the two fighters, so unlike a
/// spawned target it leaves nothing in the world.
///
/// Staging (same path for every even N): hold every slot outside the lobby,
/// admit one mint-order pair at a time to the lobby, Start each member only on
/// a permit granted once every other slot has freshly reported clear of the
/// challenge area, park a finished pair at the hold (the completed-duel Await
/// advances without sending the parking tele on that tick), and admit the
/// next pair only once both members have arrived there. A Repeat then re-teles
/// a parked slot the card walks away; the gate's zone-clearance fence fails
/// the run if that correction does not hold it (see [`DuelPairGate`]).
fn qualify_duel_arena_fleet(
    scenario: &mut scenario::Scenario,
    n: usize,
    slot: usize,
    gate: Option<&Arc<DuelPairGate>>,
) -> Result<(), String> {
    if scenario.name != "duel_arena" {
        return Ok(());
    }
    if n < 2 || !n.is_multiple_of(2) {
        return Err(format!(
            "the duel_arena benchmark pairs its slots and needs an even fleet, got N={n}"
        ));
    }
    if slot >= n {
        return Err(format!(
            "duel fleet slot {slot} is outside the even fleet N={n}"
        ));
    }
    let Some(gate) = gate else {
        return Err("duel fleet staging needs a shared pair gate".into());
    };
    if gate.n != n {
        return Err(format!(
            "duel fleet gate N={} does not match fleet N={n}",
            gate.n
        ));
    }
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, scenario::StepKind::StartScript))
        .ok_or("duel fleet qualification is missing StartScript")?;

    let hold_gate = Arc::clone(gate);
    let park_gate = Arc::clone(gate);

    scenario.steps.insert(
        start,
        duel_tele_step(
            "hold this slot outside the challenge area until its pair is admitted",
            DUEL_HOLD,
            200,
        ),
    );
    scenario.steps.insert(
        start + 1,
        scenario::Step {
            name: "wait until this pair is admitted to the challenge area",
            kind: scenario::StepKind::Await {
                evidence: "duel_pair_admitted_to_empty_lobby",
                ready: Box::new(move |_| {
                    hold_gate.mark_holding(slot);
                    hold_gate.admitted(slot)
                }),
            },
            wait: scenario::Wait {
                arm: scenario::Proof::Stat { id: 16, min: 0 },
                budget_ticks: DUEL_COMPLETED_DUEL_BUDGET_TICKS,
            },
        },
    );
    scenario.steps.insert(
        start + 2,
        duel_tele_step(
            "tele this pair into the challenge area for Start",
            DUEL_LOBBY,
            200,
        ),
    );
    let start = start + 3;
    debug_assert!(matches!(
        scenario.steps[start].kind,
        scenario::StepKind::StartScript
    ));
    scenario.steps.insert(
        start + 1,
        scenario::duel_arena_completed_duel_step(DUEL_COMPLETED_DUEL_BUDGET_TICKS),
    );
    // ArrivedNear lobby would hold on the completed-duel snapshot, so a
    // single Repeat cannot both leave the lobby and wait there for release.
    // Tele to the hold first (next pair still cannot enter), then Repeat to
    // re-tele the slot the frozen card walks away. The correction is not
    // acknowledged; the gate's zone-clearance fence is the guarantee.
    scenario.steps.insert(
        start + 2,
        duel_tele_step(
            "park this slot outside the challenge area after its duel",
            DUEL_HOLD,
            200,
        ),
    );
    scenario.steps.insert(
        start + 3,
        scenario::Step {
            name: "keep this slot out of the lobby until every pair has fought",
            kind: scenario::StepKind::Repeat {
                send: Box::new(move |c, snap| {
                    if park_gate.released() {
                        if !duel_near(snap, DUEL_LOBBY, 8) {
                            cheat(c, &tele_args(DUEL_LOBBY.level, DUEL_LOBBY.x, DUEL_LOBBY.z));
                        }
                    } else if duel_near(snap, DUEL_HOLD, 8) {
                        park_gate.mark_parked(slot);
                    } else {
                        cheat(c, &tele_args(DUEL_HOLD.level, DUEL_HOLD.x, DUEL_HOLD.z));
                    }
                    true
                }),
            },
            wait: scenario::Wait {
                arm: scenario::Proof::ArrivedNear {
                    x: DUEL_LOBBY.x,
                    z: DUEL_LOBBY.z,
                    level: DUEL_LOBBY.level,
                    radius: 8,
                },
                budget_ticks: DUEL_COMPLETED_DUEL_BUDGET_TICKS,
            },
        },
    );
    Ok(())
}

fn seed_runner(
    scenario: scenario::Scenario,
    name: &str,
    world: Option<Arc<nav::world::NavWorld>>,
    map_members: bool,
) -> Seed {
    let step_names = scenario.steps.iter().map(|step| step.name).collect();
    let deadline = if scenario.name == "duel_arena" {
        DUEL_FLEET_DEADLINE
    } else {
        Duration::from_secs(1800)
    };
    let mut runner = scenario::ScenarioRunner::with_world(scenario, world);
    runner.set_map_members(map_members);
    runner.set_live_names(&[name.to_owned()]);
    runner.set_deadline(deadline);
    Seed {
        runner,
        started: false,
        step_names,
        last_step: None,
        duel: None,
    }
}

struct ScriptCard {
    card: script::JsCard,
    bag: serde_json::Map<String, serde_json::Value>,
    siblings: Vec<(String, String)>,
    loadouts: Vec<script::Loadout>,
}

fn scenario_loadouts(settings: &scenario::ScenarioSettings) -> Vec<script::Loadout> {
    settings
        .fixture_loadouts
        .unwrap_or(&[])
        .iter()
        .map(|row| {
            row.carry
                .iter()
                .fold(script::Loadout::new(row.name), |loadout, &(item, qty)| {
                    loadout.with_carry(item, qty)
                })
        })
        .collect()
}

/// Prepared memory-benchmark run shared by panel-play and tui-play.
pub struct Run {
    pub config: Config,
    pub names: Vec<String>,
    pub vault: PathBuf,
    pub pass: String,
    frontend: &'static str,
    started: Instant,
    all_ready_since: Option<Instant>,
    all_ingame: Option<Instant>,
    qualification_complete: Option<Instant>,
    observing: Option<Instant>,
    last_sample: Option<Instant>,
    lifecycle_cycle: u64,
    stopped: bool,
    teardown: Option<Instant>,
    card: Option<ScriptCard>,
    scenario_name: Option<String>,
    output: std::fs::File,
    diagnostics: bool,
    /// Historical `BOT_MEMORY_SINGLE_RENDERER=1` only (old metadata summaries).
    pub single_renderer: bool,
    /// Requested panel draw/focus cell; TUI leaves this at [`RenderPolicy::RotatingAll`].
    pub render_policy: RenderPolicy,
    diagnostic_output: Option<std::fs::File>,
    qualification_output: std::fs::File,
    /// Which slots have ever been ready; a fleet failure names the rest.
    ready_latch: crate::memory_slots::ReadyLatch,
    /// The profile's game-data trust decision, serialized once on first poll.
    game_data: Option<serde_json::Value>,
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
    /// behavior (no second decode attempt) — and the Play's item names, which
    /// every by-name inventory proof (`Proof::Item`, `ItemAtMost`) needs: a
    /// runner without them fails such a proof closed.
    FromPlay {
        world: Option<Arc<nav::world::NavWorld>>,
        obj_names: Option<Arc<api::obj_names::ObjNames>>,
        map_members: bool,
    },
}

fn update_stable_all_ingame(
    all_ready_since: &mut Option<Instant>,
    all_ingame: &mut Option<Instant>,
    ready: usize,
    wanted: usize,
    now: Instant,
) {
    if ready == wanted {
        let since = *all_ready_since.get_or_insert(now);
        if all_ingame.is_none() && now.duration_since(since) >= READY_SETTLE {
            // Record when the stable interval began, not the end of the
            // settle hold.
            *all_ingame = Some(since);
        }
    } else {
        *all_ready_since = None;
    }
}

fn qualification_timestamp(workload: Workload, all_ingame: Instant, now: Instant) -> Instant {
    if workload == Workload::Idle {
        all_ingame
    } else {
        now
    }
}

impl Run {
    /// Mint ephemeral accounts, a throwaway vault, and an optional benchmark
    /// card, then install seed runners via [`SeedNav::LoadDefault`]. Prefer
    /// [`prepare_with_seed_nav`] from panel/TUI so seeds share `Play::world`.
    /// Idle modes need no card or RS2B0T. SeededIdle runs the Thiever setup
    /// only. Active/lifecycle runs require RS2B0T.
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
        // A frontend `main` marks the epoch first; this only guarantees the
        // epoch predates this run's own timer for any other caller.
        mark_process_start();
        // Fail closed on panel-only / conflicting flags before minting vaults.
        let render_policy = parse_render_policy(frontend)?;
        let single_renderer = std::env::var("BOT_MEMORY_SINGLE_RENDERER").as_deref() == Ok("1");
        let sustain = std::env::var("BOT_MEMORY_SUSTAIN").as_deref() == Ok("1");
        let requested_scenario =
            std::env::var("BOT_MEMORY_SCENARIO").unwrap_or_else(|_| "thiever".to_string());
        if sustain
            && (!matches!(config.workload, Workload::Active | Workload::Lifecycle)
                || requested_scenario != "thiever")
        {
            return Err("BOT_MEMORY_SUSTAIN=1 requires the active thiever scenario".into());
        }
        client::profiling::enable();
        use vault::{Profile, ProfileSettings, Vault};

        let names = crate::mint_live_names(config.n);
        let diagnostics = std::env::var("BOT_MEMORY_DIAGNOSTICS").as_deref() == Ok("1");
        if diagnostics {
            crate::memory_diagnostics::enable(&names);
        }
        let pass = crate::live_vault_passphrase();
        let dir = std::env::temp_dir().join(format!("274bot-memory-{}", names[0]));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join("vault");
        let mut vault = Vault::create(&path, &pass).map_err(|e| e.to_string())?;

        for (i, name) in names.iter().enumerate() {
            let mut settings = ProfileSettings {
                auto_login: true,
                ..ProfileSettings::default()
            };
            match render_policy {
                RenderPolicy::Stress50 => {
                    settings.lowmem = true;
                    settings.raster = if i == 0 {
                        vault::RasterMode::Gpu
                    } else {
                        vault::RasterMode::Off
                    };
                }
                RenderPolicy::Stress50Full => {
                    settings.lowmem = true;
                    settings.raster = vault::RasterMode::Gpu;
                }
                _ => {}
            }
            vault
                .upsert(Profile {
                    username: name.clone(),
                    password: name.clone().into(),
                    uid: 274_900_000 + i as i32,
                    settings,
                })
                .map_err(|e| e.to_string())?;
        }
        // Seeds install after Play exists (or via prepare's LoadDefault path).
        *SEEDS.lock().unwrap() = Some(HashMap::new());

        let scenario_name = if matches!(config.workload, Workload::Idle | Workload::SeededIdle) {
            None
        } else {
            Some(requested_scenario)
        };
        let card = if let Some(scenario_name) = scenario_name.as_deref() {
            let benchmark = scenario::get(scenario_name)
                .ok_or_else(|| format!("unknown BOT_MEMORY_SCENARIO {scenario_name}"))?;
            let card_name = benchmark.settings.start_script.ok_or_else(|| {
                format!("BOT_MEMORY_SCENARIO {scenario_name} has no catalog script")
            })?;
            let root = std::env::var_os("RS2B0T")
                .map(PathBuf::from)
                .ok_or("RS2B0T is required for the active memory benchmark")?;
            let mut library = script::JsLibrary::new(dir.join("scripts.json"));
            library.register_rs2b0t(&root, &dir.join("rs2b0t-path"))?;
            library.ensure_js(script::ScriptSource::Catalog, card_name)?;
            let card = library
                .get(script::ScriptSource::Catalog, card_name)
                .cloned()
                .ok_or_else(|| format!("missing catalog card {card_name}"))?;
            let inject = scenario::settings_inject_map(benchmark.settings.script_settings_inject);
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
                    api_family: Some(card.api_family.as_str().into()),
                },
            )?;
            Some(ScriptCard {
                card,
                bag,
                siblings,
                loadouts: scenario_loadouts(&benchmark.settings),
            })
        } else {
            None
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
            .write(true)
            .create_new(true)
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
        let ready_latch = crate::memory_slots::ReadyLatch::new(&names);
        Ok(Self {
            config,
            names,
            vault: path,
            pass,
            frontend,
            started: Instant::now(),
            all_ready_since: None,
            all_ingame: None,
            qualification_complete: None,
            observing: None,
            last_sample: None,
            lifecycle_cycle: 0,
            stopped: false,
            teardown: None,
            card,
            scenario_name,
            output,
            diagnostics,
            diagnostic_output,
            qualification_output,
            single_renderer,
            render_policy,
            ready_latch,
            game_data: None,
        })
    }

    /// Install the selected scenario's seed runners for non-idle workloads.
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
        let (seed_world, obj_names, map_members) = match seed_nav {
            SeedNav::LoadDefault => (
                nav::world::NavWorld::load_pack(&scenario::default_pack_path())
                    .ok()
                    .map(Arc::new),
                None,
                false,
            ),
            SeedNav::FromPlay {
                world,
                obj_names,
                map_members,
            } => (world, obj_names, map_members),
        };
        let mut seeds = HashMap::new();
        let duel_gate = (self.scenario_name.as_deref() == Some("duel_arena"))
            .then(|| DuelPairGate::new(self.config.n));
        for (slot, name) in self.names.iter().enumerate() {
            let mut scenario = if self.config.workload == Workload::SeededIdle {
                seeded_idle_scenario()
            } else if self.scenario_name.as_deref() == Some("thiever")
                && std::env::var("BOT_MEMORY_SUSTAIN").as_deref() == Ok("1")
            {
                scenario::thiever_sustained_scenario()
            } else {
                let scenario_name = self
                    .scenario_name
                    .as_deref()
                    .ok_or("active benchmark is missing its scenario")?;
                scenario::get(scenario_name)
                    .ok_or_else(|| format!("missing benchmark scenario {scenario_name}"))?
            };
            widen_fleet_post_start_waits(&mut scenario, self.config.n);
            qualify_contentious_moss_fleet(&mut scenario, self.config.n)?;
            qualify_duel_arena_fleet(&mut scenario, self.config.n, slot, duel_gate.as_ref())?;
            scenario.settings.terminal_shot = None;
            let mut seed = seed_runner(scenario, name, seed_world.clone(), map_members);
            seed.duel = duel_gate.as_ref().map(|gate| DuelSlot {
                gate: Arc::clone(gate),
                slot,
            });
            if let Some(names) = &obj_names {
                seed.runner.set_obj_names(Arc::clone(names));
            }
            seeds.insert(name.clone(), Arc::new(Mutex::new(seed)));
        }
        *SEEDS.lock().unwrap() = Some(seeds);
        Ok(())
    }

    pub fn has_script_card(&self) -> bool {
        self.card.is_some()
    }

    fn start_script(&self, play: &Play, name: &str) -> Result<(), String> {
        let Some(card) = &self.card else {
            return Ok(());
        };
        if self.scenario_name.as_deref() == Some("thiever")
            && std::env::var("BOT_MEMORY_SUSTAIN").as_deref() == Ok("1")
        {
            let mut bag = card.bag.clone();
            bag.insert("banking".into(), serde_json::json!("Auto"));
            bag.insert("loadout".into(), serde_json::json!("Memory food"));
            bag.insert("foodWithdraw".into(), serde_json::json!(22));
            bag.insert("bankAtFood".into(), serde_json::json!(3));
            let slot = crate::script_slot_or_insert(&play.scripts, name);
            let mut slot = slot.lock().unwrap();
            slot.start_load_with_loadouts_and_game_data(
                card.card.js.clone(),
                card.card.shape,
                card.siblings.clone(),
                &[script::Loadout::new("Memory food").with_carry("Lobster", 1)],
                play.game_data(),
                play.named_banks(),
            )?;
            slot.post_settings_bag(&bag);
            drop(slot);
            play.wake(name);
            return Ok(());
        }
        // Fixtures must not read the operator's saved loadouts.
        let slot = crate::script_slot_or_insert(&play.scripts, name);
        let mut slot = slot.lock().unwrap();
        slot.start_load_with_loadouts_and_game_data(
            card.card.js.clone(),
            card.card.shape,
            card.siblings.clone(),
            &card.loadouts,
            play.game_data(),
            play.named_banks(),
        )?;
        slot.post_settings_bag(&card.bag);
        drop(slot);
        play.wake(name);
        Ok(())
    }

    /// Drive warmup/observe/lifecycle. `Ok(true)` when teardown finished. A
    /// failure is recorded per slot before it is returned, because both
    /// frontends exit on it.
    pub fn poll(&mut self, play: &Play) -> Result<bool, String> {
        let result = self.poll_inner(play);
        if let Err(error) = &result {
            self.record_failure(play, error);
        }
        result
    }

    fn poll_inner(&mut self, play: &Play) -> Result<bool, String> {
        let now = Instant::now();
        let statuses = play.statuses();
        self.ready_latch.observe(&statuses, now);
        let ready = statuses
            .iter()
            .filter(|s| s.ingame && s.scene_state == 2)
            .count();

        update_stable_all_ingame(
            &mut self.all_ready_since,
            &mut self.all_ingame,
            ready,
            self.config.n,
            now,
        );

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
                let (status, on_start, already_started, duel) = {
                    let mut seed = seed_arc.lock().unwrap();
                    let status = seed.runner.status();
                    if let scenario::RunnerStatus::Running { step, .. } = &status {
                        seed.last_step = Some(*step);
                    }
                    (
                        status,
                        seed.runner.on_start_script(),
                        seed.started,
                        seed.duel.clone(),
                    )
                };
                if let Some(failure) = duel.as_ref().and_then(|duel| duel.gate.failure()) {
                    return Err(self.duel_fleet_failure(play, failure)?);
                }
                match status {
                    scenario::RunnerStatus::Failed(msg) => {
                        let failure = format!("benchmark seed/proof failed for {name}: {msg}");
                        if self.diagnostics {
                            self.write_diagnostics(play, Some(&failure))?;
                        }
                        return Err(failure);
                    }
                    scenario::RunnerStatus::Passed => {
                        seeded += 1;
                        proved += 1;
                    }
                    _ if on_start => {
                        seeded += 1;
                        if !already_started {
                            // A duel slot Starts only on a permit the
                            // admission owner grants after every other slot
                            // has reported clear since this request.
                            let permitted =
                                match duel.as_ref().map(|duel| duel.gate.poll_start(duel.slot)) {
                                    None | Some(StartPermit::Granted) => true,
                                    Some(StartPermit::Wait) => false,
                                    Some(StartPermit::Refused(failure)) => {
                                        return Err(self.duel_fleet_failure(play, failure)?);
                                    }
                                };
                            if permitted {
                                self.start_script(play, name)?;
                                seed_arc.lock().unwrap().started = true;
                            }
                        }
                    }
                    _ => {}
                }
                if let Some(error) = play.script_last_error(name) {
                    let failure = format!("{name}: {error}");
                    if self.diagnostics {
                        self.write_diagnostics(play, Some(&failure))?;
                    }
                    return Err(failure);
                }
            }
        }

        let active = self
            .names
            .iter()
            .filter(|name| play.script_state(name) == script::RunState::Running)
            .count();

        // Unseeded idle: stable ingame readiness only. Seeded idle: seed
        // completed, no scripts. Active/lifecycle: all ready, seeded,
        // XP-proved, and scripts up.
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

        if self.qualification_complete.is_none() && established {
            if let Some(all_ingame) = self.all_ingame {
                self.qualification_complete = Some(qualification_timestamp(
                    self.config.workload,
                    all_ingame,
                    now,
                ));
            }
        }
        let qualify_bound = if self.scenario_name.as_deref() == Some("duel_arena") {
            DUEL_FLEET_DEADLINE
        } else {
            Duration::from_secs(1800)
        };
        if self.qualification_complete.is_none() && self.started.elapsed() > qualify_bound {
            return Err(format!(
                "blocked: ready={ready} seeded={seeded} proved={proved} wanted={} ever_ready={}",
                self.config.n,
                self.ready_latch.ever()
            ));
        }
        if self.observing.is_none()
            && self
                .qualification_complete
                .is_some_and(|t| t.elapsed() >= self.config.warmup)
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
            let sum = |key: &str| metrics.iter().filter_map(|m| m[key].as_u64()).sum::<u64>();
            let live = sum("v8_live");
            let sampled = metrics
                .iter()
                .filter(|m| m["v8_live"] == 1 && m["v8_heap_samples"].as_u64().unwrap_or(0) > 0)
                .count() as u64;
            let gpu = client::profiling::gpu_bytes();
            let sample = Sample {
                frontend: self.frontend.into(),
                n: self.config.n,
                workload: self.config.workload,
                elapsed_s: self.started.elapsed().as_secs_f64(),
                phase: if self.teardown.is_some() {
                    "teardown".into()
                } else if self.observing.is_some() {
                    "observe".into()
                } else if self.qualification_complete.is_some() {
                    "warmup".into()
                } else {
                    "seed".into()
                },
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
                rust_allocated_bytes: (!cfg!(feature = "memory-profile-no-alloc"))
                    .then_some(alloc_bytes),
                rust_live_bytes: (!cfg!(feature = "memory-profile-no-alloc")).then_some(live_bytes),
                snapshot_inflight_bytes: Some(sum("snapshot_inflight_bytes")),
                snapshot_inflight_capacity: Some(sum("snapshot_inflight_capacity")),
                v8_used_bytes: (live == sampled).then(|| sum("v8_used_bytes")),
                v8_total_bytes: (live == sampled).then(|| sum("v8_total_bytes")),
                gpu_tracked_bytes: Some(gpu.buffers + gpu.textures),
            };
            let mut value = sample.to_json();
            let cpu = process_cpu_seconds();
            value["process_cpu_user_s"] = cpu.map(|v| v.0).into();
            value["process_cpu_system_s"] = cpu.map(|v| v.1).into();
            value["allocation_counting"] = (!cfg!(feature = "memory-profile-no-alloc")).into();
            value["diagnostic_sidecar"] = self.diagnostics.into();
            value["benchmark_scenario"] = self.scenario_name.clone().into();
            value["contention_qualification"] = (self.config.n > 1
                && self.scenario_name.as_deref() == Some("moss_giant_bank_start"))
            .into();
            value["all_bots_ingame_scene2_s"] = self
                .all_ingame
                .map(|ready| ready.duration_since(self.started).as_secs_f64())
                .into();
            value["qualification_complete_s"] = self
                .qualification_complete
                .map(|ready| ready.duration_since(self.started).as_secs_f64())
                .into();
            let host_timings = host::performance_profile::snapshots();
            value["host_profile_slots"] = host_timings.len().into();
            value["host_loop_total_ns"] = host_timings
                .iter()
                .map(|timing| timing.loop_ns)
                .sum::<u64>()
                .into();
            value["host_observe_total_ns"] = host_timings
                .iter()
                .map(|timing| timing.observe_ns)
                .sum::<u64>()
                .into();
            value["host_raster_total_ns"] = host_timings
                .iter()
                .map(|timing| timing.raster_ns)
                .sum::<u64>()
                .into();
            value["panel_frame_histogram_ms"] = serde_json::json!(panel_frame_histogram());
            // Requested mode metadata only — not observed GPU/cadence proof.
            value["single_renderer"] = self.single_renderer.into();
            value["render_policy"] = self.render_policy.as_str().into();
            value["render_policy_requested"] = true.into();
            value["gpu_buffer_bytes"] = gpu.buffers.into();
            value["gpu_texture_bytes"] = gpu.textures.into();
            value["gpu_peak_tracked_bytes"] = gpu.peak.into();
            value["v8_live_isolates"] = live.into();
            value["v8_sampled_isolates"] = sampled.into();
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            value["v8_max_sample_age_ms"] = metrics
                .iter()
                .filter(|m| m["v8_live"] == 1)
                .filter_map(|m| m["v8_updated_ms"].as_u64())
                .map(|t| now_ms.saturating_sub(t))
                .max()
                .map(serde_json::Value::from)
                .unwrap_or(serde_json::Value::Null);
            for key in ["script_tick_count", "script_tick_total_ns"] {
                value[key] = sum(key).into();
            }
            value["script_tick_max_ns"] = metrics
                .iter()
                .filter_map(|m| m["script_tick_max_ns"].as_u64())
                .max()
                .unwrap_or(0)
                .into();
            value["snapshot_sum_isolate_peak_capacity"] = sum("snapshot_peak_capacity").into();
            for (key, counter) in [
                ("client_tick", &client::profiling::CLIENT_TICK),
                ("ui_draw", &client::profiling::UI_DRAW),
            ] {
                let (count, total, max) = counter.read();
                value[format!("{key}_count")] = count.into();
                value[format!("{key}_total_ns")] = total.into();
                value[format!("{key}_max_ns")] = max.into();
            }
            let (ui_frame_count, ui_frame_total_ns, _) = client::profiling::UI_FRAME.read();
            value["ui_frame_count"] = ui_frame_count.into();
            value["ui_frame_total_ns"] = ui_frame_total_ns.into();
            value["ui_frame_max_ns"] = take_panel_frame_interval_max_ns().into();
            value["ui_frame_max_scope"] = "sample-interval".into();
            crate::memory_startup::add_sample_fields(&mut value, self.started);
            value["ready_ever"] = self.ready_latch.ever().into();
            let seed_counts = (self.config.workload != Workload::Idle).then_some((seeded, proved));
            value["seeded"] = seed_counts.map(|(seeded, _)| seeded).into();
            value["proved"] = seed_counts.map(|(_, proved)| proved).into();
            value["game_data"] = self.game_data_json(play);
            writeln!(self.output, "{value}").map_err(|e| e.to_string())?;
            if self.diagnostics {
                self.write_diagnostics(play, None)?;
            }
            self.last_sample = Some(now);
        }

        Ok(self
            .teardown
            .is_some_and(|t| t.elapsed() >= self.config.teardown))
    }

    /// The profile's game-data trust decision, serialized once.
    fn game_data_json(&mut self, play: &Play) -> serde_json::Value {
        self.game_data
            .get_or_insert_with(|| {
                play.server_profile()
                    .map_or(serde_json::Value::Null, |profile| {
                        profile.game_data_status().to_json()
                    })
            })
            .clone()
    }

    /// One outcome per slot: whether it ever reached `ingame && scene_state
    /// == 2` and its last observed session, scenario and script state.
    fn slot_outcomes(&self, play: &Play) -> Vec<serde_json::Value> {
        use crate::memory_slots::{slot_outcome, slot_qualified, ScriptView, SeedView};
        let now = Instant::now();
        let statuses = play.statuses();
        let seeds = SEEDS.lock().unwrap();
        self.names
            .iter()
            .map(|name| {
                let seed = seeds
                    .as_ref()
                    .and_then(|map| map.get(name))
                    .map(|seed| seed.lock().unwrap());
                let status = statuses.iter().find(|s| &s.username == name);
                let script_state = play.script_state(name);
                let qualified_now = slot_qualified(
                    self.config.workload,
                    self.card.is_some(),
                    status.is_some_and(|s| s.ingame && s.scene_state == 2),
                    seed.as_ref()
                        .map(|seed| (seed.runner.status(), seed.runner.on_start_script()))
                        .as_ref()
                        .map(|(status, on_start)| (status, *on_start)),
                    script_state == script::RunState::Running,
                );
                slot_outcome(
                    name,
                    self.ready_latch
                        .first_ready(name)
                        .map(|at| at.saturating_duration_since(self.started).as_secs_f64()),
                    status,
                    now,
                    seed.as_ref().map(|seed| SeedView {
                        status: seed.runner.status(),
                        started: seed.started,
                        step_names: &seed.step_names,
                        last_step: seed.last_step,
                    }),
                    ScriptView {
                        state: format!("{script_state:?}"),
                        error: play.script_last_error(name),
                        runtime: play.memory_script_progress(name),
                    },
                    qualified_now,
                )
            })
            .collect()
    }

    // Two boundary reads preserve script progress evidence when verbose
    // diagnostic collection is disabled. Never drains logs or sends actions.
    fn write_qualification(&mut self, play: &Play, phase: &str) -> Result<(), String> {
        let slots = self.slot_outcomes(play);
        let mut value = serde_json::json!({
            "phase": phase,
            "elapsed_s": self.started.elapsed().as_secs_f64(),
            "ready_ever": self.ready_latch.ever(),
            "slots": slots,
        });
        if let Some(gate) = duel_gate_summary() {
            value["duel_gate"] = gate;
        }
        writeln!(self.qualification_output, "{value}").map_err(|e| e.to_string())
    }

    /// The duel fleet's recorded failure, named with its slot's account,
    /// after the diagnostics it owes when they are enabled.
    fn duel_fleet_failure(
        &mut self,
        play: &Play,
        failure: DuelFleetFailure,
    ) -> Result<String, String> {
        let failure = format!("{} ({})", failure.message, self.names[failure.slot]);
        if self.diagnostics {
            self.write_diagnostics(play, Some(&failure))?;
        }
        Ok(failure)
    }

    /// The fail-closed record: which slots ever qualified and where each of
    /// the others stopped. Best effort, and never replaces the failure it
    /// documents.
    fn record_failure(&mut self, play: &Play, error: &str) {
        let slots = self.slot_outcomes(play);
        let ready_now = slots
            .iter()
            .filter(|slot| slot["ingame_scene2_now"] == true)
            .count();
        let mut value = serde_json::json!({
            "phase": "failed",
            "error": error,
            "elapsed_s": self.started.elapsed().as_secs_f64(),
            "wanted": self.config.n,
            "ready_now": ready_now,
            "ready_ever": self.ready_latch.ever(),
            "slots": slots,
        });
        if let Some(gate) = duel_gate_summary() {
            value["duel_gate"] = gate;
        }
        if let Err(write_error) = writeln!(self.qualification_output, "{value}") {
            eprintln!("memory benchmark: could not record slot outcomes: {write_error}");
        }
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
    use client::client::{Client, ClientPlayer};
    use client::io::ServerProt;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    /// Env mutation is process-global; serialize these tests.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

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
    fn stable_readiness_discards_the_pre_hop_ready_pulse() {
        let base = Instant::now();
        let mut since = None;
        let mut ready = None;
        update_stable_all_ingame(&mut since, &mut ready, 1, 1, base);
        update_stable_all_ingame(&mut since, &mut ready, 1, 1, base + Duration::from_secs(1));
        assert!(ready.is_none(), "one-second ready pulse must not latch");
        update_stable_all_ingame(
            &mut since,
            &mut ready,
            0,
            1,
            base + Duration::from_millis(1100),
        );
        let settled = base + Duration::from_secs(3);
        update_stable_all_ingame(&mut since, &mut ready, 1, 1, settled);
        update_stable_all_ingame(&mut since, &mut ready, 1, 1, settled + READY_SETTLE);
        assert_eq!(ready, Some(settled));
    }

    #[test]
    fn seeded_idle_qualification_records_seed_completion_not_ingame_readiness() {
        let all_ingame = Instant::now();
        let seeded = all_ingame + Duration::from_secs(7);
        assert_eq!(
            qualification_timestamp(Workload::SeededIdle, all_ingame, seeded),
            seeded
        );
        assert_eq!(
            qualification_timestamp(Workload::Idle, all_ingame, seeded),
            all_ingame
        );
    }

    #[test]
    fn panel_frame_interval_max_resets_at_each_sample() {
        PANEL_FRAME_INTERVAL_MAX_NS.store(0, Relaxed);
        PANEL_FRAME_INTERVAL_MAX_NS.fetch_max(12, Relaxed);
        PANEL_FRAME_INTERVAL_MAX_NS.fetch_max(7, Relaxed);
        assert_eq!(take_panel_frame_interval_max_ns(), 12);
        assert_eq!(take_panel_frame_interval_max_ns(), 0);
    }

    #[test]
    fn parse_n_accepts_campaign_and_legacy_sizes() {
        for n in [1usize, 10, 16, 32, 50, 128] {
            assert_eq!(parse_n(&n.to_string()).unwrap(), n);
        }
    }

    #[test]
    fn parse_n_rejects_invalid() {
        for s in ["0", "2", "49", "51", "", "-1"] {
            assert!(parse_n(s).is_err(), "expected err for {s:?}");
        }
    }

    fn clear_render_env() -> EnvGuard {
        EnvGuard::clear(&["BOT_MEMORY_SINGLE_RENDERER", "BOT_MEMORY_RENDER_POLICY"])
    }

    #[test]
    fn parse_render_policy_defaults_rotating_all() {
        let _lock = ENV_LOCK.lock().unwrap();
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
        let _lock = ENV_LOCK.lock().unwrap();
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
        let _lock = ENV_LOCK.lock().unwrap();
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
        std::env::set_var("BOT_MEMORY_RENDER_POLICY", "stress50");
        assert_eq!(
            parse_render_policy("panel").unwrap(),
            RenderPolicy::Stress50
        );
        std::env::set_var("BOT_MEMORY_RENDER_POLICY", "stress50-full");
        assert_eq!(
            parse_render_policy("panel").unwrap(),
            RenderPolicy::Stress50Full
        );
        assert!(parse_render_policy("tui").is_err());
    }

    #[test]
    fn parse_render_policy_rejects_conflict_and_unknown() {
        let _lock = ENV_LOCK.lock().unwrap();
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
        let _lock = ENV_LOCK.lock().unwrap();
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
        let _lock = ENV_LOCK.lock().unwrap();
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
        let _lock = ENV_LOCK.lock().unwrap();
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
        let _lock = ENV_LOCK.lock().unwrap();
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
        let _lock = ENV_LOCK.lock().unwrap();
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
    fn prepare_rejects_sustain_for_non_thiever_scenario() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::clear(&[
            "RS2B0T",
            "BOT_MEMORY_OUTPUT",
            "BOT_MEMORY_SCENARIO",
            "BOT_MEMORY_SUSTAIN",
        ]);
        std::env::set_var("BOT_MEMORY_SCENARIO", "moss_giant_bank_start");
        std::env::set_var("BOT_MEMORY_SUSTAIN", "1");
        let err = match Run::prepare_unseeded(unit_config(1, Workload::Active), "tui") {
            Err(error) => error,
            Ok(_) => panic!("expected non-thiever sustain error"),
        };
        assert_eq!(
            err,
            "BOT_MEMORY_SUSTAIN=1 requires the active thiever scenario"
        );
    }

    #[test]
    fn config_from_env_none_without_bot_memory_n() {
        let _lock = ENV_LOCK.lock().unwrap();
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
        let _lock = ENV_LOCK.lock().unwrap();
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
        let _lock = ENV_LOCK.lock().unwrap();
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
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::clear(&[
            "BOT_MEMORY_N",
            "BOT_MEMORY_WORKLOAD",
            "BOT_MEMORY_WARMUP_S",
            "BOT_MEMORY_OBSERVE_S",
            "BOT_MEMORY_TEARDOWN_S",
        ]);
        std::env::set_var("BOT_MEMORY_N", "1");
        std::env::set_var("BOT_MEMORY_WARMUP_S", "0");
        assert!(Config::from_env().is_err());
        std::env::remove_var("BOT_MEMORY_WARMUP_S");
        std::env::set_var("BOT_MEMORY_OBSERVE_S", "0");
        assert!(Config::from_env().is_err());
        std::env::remove_var("BOT_MEMORY_OBSERVE_S");
        std::env::set_var("BOT_MEMORY_TEARDOWN_S", "0");
        assert!(Config::from_env().is_err());
    }

    #[test]
    fn config_from_env_defaults_idle_and_durations() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::clear(&[
            "BOT_MEMORY_N",
            "BOT_MEMORY_WORKLOAD",
            "BOT_MEMORY_WARMUP_S",
            "BOT_MEMORY_OBSERVE_S",
            "BOT_MEMORY_TEARDOWN_S",
        ]);
        std::env::set_var("BOT_MEMORY_N", "32");
        let cfg = Config::from_env().unwrap().expect("Some");
        assert_eq!(cfg.n, 32);
        assert_eq!(cfg.workload, Workload::Idle);
        assert_eq!(cfg.warmup.as_secs(), 120);
        assert_eq!(cfg.observe.as_secs(), 600);
        assert_eq!(cfg.teardown.as_secs(), 60);
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
            teardown: Duration::from_secs(1),
        }
    }

    #[test]
    fn prepare_idle_mints_n_names_vault_under_temp_no_card() {
        let _lock = ENV_LOCK.lock().unwrap();
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
                origin: api::snapshot::WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                width: 1,
                height: 1,
                walk: vec![0; 4],
                blocked: vec![0],
                flags: None,
            },
            Default::default(),
            vec![],
        ));
        let mut a = seed_runner(seeded_idle_scenario(), "seed_a", Some(world.clone()), false);
        let b = seed_runner(seeded_idle_scenario(), "seed_b", Some(world.clone()), false);
        assert_eq!(
            Arc::strong_count(&world),
            3,
            "both runners must retain the injected world"
        );
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
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::clear(&[
            "RS2B0T",
            "BOT_MEMORY_OUTPUT",
            "BOT_MEMORY_N",
            "BOT_MEMORY_WORKLOAD",
        ]);
        let world = Arc::new(nav::world::NavWorld::from_parts(
            nav::collision::WorldCollision {
                origin: api::snapshot::WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                width: 1,
                height: 1,
                walk: vec![0; 4],
                blocked: vec![0],
                flags: None,
            },
            Default::default(),
            vec![],
        ));
        let run = Run::prepare_unseeded(unit_config(2, Workload::SeededIdle), "unit")
            .expect("unseeded prepare");
        run.bind_seed_nav(SeedNav::FromPlay {
            world: Some(world.clone()),
            obj_names: None,
            map_members: false,
        })
        .expect("bind play world");
        let seeds = SEEDS.lock().unwrap();
        let map = seeds.as_ref().expect("seeds installed");
        assert_eq!(map.len(), 2);
        for name in &run.names {
            let seed = map.get(name).expect("named seed").lock().unwrap();
            let shared = seed
                .runner
                .shared_world()
                .expect("seed must hold play world");
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
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::clear(&["RS2B0T", "BOT_MEMORY_OUTPUT"]);
        let run = Run::prepare_unseeded(unit_config(1, Workload::SeededIdle), "unit")
            .expect("unseeded prepare");
        // Play had no pack: FromPlay(None) must not attempt a second decode.
        run.bind_seed_nav(SeedNav::FromPlay {
            world: None,
            obj_names: None,
            map_members: false,
        })
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
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::clear(&["RS2B0T", "BOT_MEMORY_OUTPUT"]);
        let world = Arc::new(nav::world::NavWorld::from_parts(
            nav::collision::WorldCollision {
                origin: api::snapshot::WorldTile {
                    x: 1,
                    z: 2,
                    level: 0,
                },
                width: 1,
                height: 1,
                walk: vec![0; 4],
                blocked: vec![0],
                flags: None,
            },
            Default::default(),
            vec![],
        ));
        let run = Run::prepare_with_seed_nav(
            unit_config(1, Workload::SeededIdle),
            "unit",
            SeedNav::FromPlay {
                world: Some(world.clone()),
                obj_names: None,
                map_members: false,
            },
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
    fn moss_fleet_uses_engagement_without_weakening_single_bot_xp_proof() {
        let mut single = scenario::get("moss_giant_bank_start").expect("moss scenario");
        qualify_contentious_moss_fleet(&mut single, 1).expect("single qualifier");
        assert!(single
            .steps
            .iter()
            .any(|step| { step.wait.arm == scenario::Proof::FreshStatXpGain { id: 2, min: 1 } }));

        let mut fleet = scenario::get("moss_giant_bank_start").expect("moss scenario");
        qualify_contentious_moss_fleet(&mut fleet, 10).expect("fleet qualifier");
        let engagement = fleet
            .steps
            .iter()
            .find(|step| {
                step.name
                    == "watch the local player target a Moss giant after the startup bank return"
            })
            .expect("post-return engagement");
        assert_eq!(
            engagement.wait.arm,
            scenario::Proof::LocalTargetingNpcName { name: "Moss giant" }
        );
        assert_eq!(
            fleet.proof,
            scenario::Proof::StatXpGain { id: 2, min: 1 },
            "fleet qualification must retain the final post-Start Strength XP proof"
        );
    }

    /// The Duel Arena card pairs its own slots: an even fleet stages one
    /// mint-order pair at a time (hold outside the lobby, admit, Start, completed
    /// duel, park at the hold, keep parked until every pair has fought)
    /// and keeps the scenario's proof. A fleet that cannot pair, or another
    /// scenario, is refused or untouched. N=2, N=10 and N=50 share that path.
    #[test]
    fn duel_fleets_pair_up_and_owe_a_completed_duel_after_start() {
        let baseline = scenario::get("duel_arena").expect("duel scenario");
        let start = baseline
            .steps
            .iter()
            .position(|step| matches!(step.kind, scenario::StepKind::StartScript))
            .expect("Start");
        assert_eq!(start + 1, baseline.steps.len(), "Start is the last step");

        for n in [2, 10, 50] {
            let gate = DuelPairGate::new(n);
            let mut fleet = scenario::get("duel_arena").expect("duel scenario");
            qualify_duel_arena_fleet(&mut fleet, n, 0, Some(&gate)).expect("even fleet");
            assert_eq!(fleet.steps.len(), baseline.steps.len() + 6, "N={n}");
            assert_eq!(
                fleet.steps[start].name,
                "hold this slot outside the challenge area until its pair is admitted"
            );
            assert_eq!(
                fleet.steps[start + 1].name,
                "wait until this pair is admitted to the challenge area"
            );
            assert_eq!(
                fleet.steps[start + 2].name,
                "tele this pair into the challenge area for Start"
            );
            assert!(
                matches!(fleet.steps[start + 3].kind, scenario::StepKind::StartScript),
                "N={n}"
            );
            let watch = &fleet.steps[start + 4];
            assert!(matches!(watch.kind, scenario::StepKind::Await { .. }));
            assert_eq!(watch.wait.budget_ticks, DUEL_COMPLETED_DUEL_BUDGET_TICKS);
            assert_eq!(
                fleet.steps[start + 5].name,
                "park this slot outside the challenge area after its duel"
            );
            assert!(matches!(
                fleet.steps[start + 6].kind,
                scenario::StepKind::Repeat { .. }
            ));
            assert_eq!(fleet.proof, baseline.proof, "N={n}");
        }

        for n in [1, 3, 49] {
            let mut fleet = scenario::get("duel_arena").expect("duel scenario");
            let error = qualify_duel_arena_fleet(&mut fleet, n, 0, None).expect_err("unpairable");
            assert!(error.contains("even fleet"), "N={n}: {error}");
            assert_eq!(fleet.steps.len(), baseline.steps.len(), "N={n}");
        }

        let mut moss = scenario::get("moss_giant_bank_start").expect("moss scenario");
        let steps = moss.steps.len();
        qualify_duel_arena_fleet(&mut moss, 1, 0, None).expect("other scenarios pass through");
        assert_eq!(moss.steps.len(), steps);
    }

    fn duel_tile(tile: WorldTile) -> (i32, i32, i32) {
        (tile.x, tile.z, tile.level)
    }

    fn report_at(gate: &DuelPairGate, slots: &[usize], tile: WorldTile) {
        for &slot in slots {
            gate.observe(slot, duel_tile(tile));
        }
    }

    /// One step inside the zone clearance, still outside the challenge area.
    const DUEL_NEAR_ZONE: WorldTile = WorldTile {
        x: DUEL_ZONE.0 - DUEL_ZONE_CLEARANCE_STEPS + 1,
        z: 3230,
        level: 0,
    };

    /// A four-slot fleet whose pair 0 has fought and parked, so pair 1 is
    /// admitted. Every slot's latest report is clear at the hold and was made
    /// before any pair-1 Start request.
    fn duel_gate_with_pair1_admitted() -> Arc<DuelPairGate> {
        let gate = DuelPairGate::new(4);
        for slot in 0..4 {
            gate.mark_holding(slot);
        }
        for slot in [0, 1] {
            assert_eq!(gate.poll_start(slot), StartPermit::Wait);
        }
        report_at(&gate, &[2, 3], DUEL_HOLD);
        for slot in [0, 1] {
            assert_eq!(gate.poll_start(slot), StartPermit::Granted);
        }
        report_at(&gate, &[0, 1], DUEL_PEN);
        report_at(&gate, &[0, 1], DUEL_HOLD);
        gate.mark_parked(0);
        gate.mark_parked(1);
        report_at(&gate, &[0, 1, 2, 3], DUEL_HOLD);
        assert!(gate.admitted(2) && gate.admitted(3));
        gate
    }

    /// Poll `slot`'s Start until it is decided, failing on a hang.
    fn decided_start(gate: &DuelPairGate, slot: usize) -> StartPermit {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match gate.poll_start(slot) {
                StartPermit::Wait => {
                    assert!(
                        Instant::now() < deadline,
                        "slot {slot}'s Start never decided"
                    );
                    std::thread::yield_now();
                }
                decided => return decided,
            }
        }
    }

    fn assert_slot0_breach(permit: StartPermit) {
        let StartPermit::Refused(failure) = permit else {
            panic!("expected slot 0's breach to refuse the Start, got {permit:?}");
        };
        assert_eq!(failure.slot, 0);
        assert!(
            failure.message.contains("clearance breach") && failure.message.contains("slot 0"),
            "{}",
            failure.message
        );
    }

    /// Pair 1 cannot leave the hold until both members of pair 0 have parked
    /// at it. A Start needs a permit, granted in request order and only once
    /// every slot outside the pair has reported clear since the request. The
    /// admitted pair's own trips to the lobby and the pens are not breaches,
    /// and the fence comes down only once the last pair has parked and every
    /// slot has reported clear since.
    #[test]
    fn duel_pair_gate_admits_one_pair_after_both_are_parked() {
        let gate = DuelPairGate::new(4);
        assert!(!gate.admitted(0), "hold is empty");
        for slot in 0..4 {
            gate.mark_holding(slot);
        }
        assert!(gate.admitted(0) && gate.admitted(1));
        assert!(!gate.admitted(2), "pair 1 waits on pair 0");

        assert_eq!(
            gate.poll_start(0),
            StartPermit::Wait,
            "nobody has reported since the request"
        );
        assert_eq!(gate.poll_start(1), StartPermit::Wait);
        report_at(&gate, &[2], DUEL_HOLD);
        assert_eq!(
            gate.poll_start(0),
            StartPermit::Wait,
            "slot 3 has not reported"
        );
        report_at(&gate, &[3], DUEL_HOLD);
        assert_eq!(
            gate.poll_start(1),
            StartPermit::Wait,
            "slot 0 heads the queue"
        );
        assert_eq!(gate.poll_start(0), StartPermit::Granted);
        assert_eq!(
            gate.poll_start(1),
            StartPermit::Granted,
            "the same reports postdate its request"
        );

        report_at(&gate, &[0, 1], DUEL_LOBBY);
        report_at(&gate, &[0, 1], DUEL_PEN);
        report_at(&gate, &[0, 1], DUEL_LOBBY);
        assert_eq!(gate.failure(), None, "the admitted pair may leave the hold");
        assert!(
            !gate.admitted(2),
            "a pair back from its duel is still in the lobby"
        );

        report_at(&gate, &[0], DUEL_HOLD);
        gate.mark_parked(0);
        assert!(!gate.admitted(2), "one parked fighter is not a pair");
        gate.mark_parked(0);
        report_at(&gate, &[1], DUEL_HOLD);
        gate.mark_parked(1);
        assert!(gate.admitted(2) && gate.admitted(3), "pair 0 is parked");
        assert!(!gate.admitted(0), "a fought pair is not admitted again");

        assert_eq!(gate.poll_start(2), StartPermit::Wait);
        report_at(&gate, &[0, 1], DUEL_HOLD);
        assert_eq!(gate.poll_start(2), StartPermit::Granted);
        assert_eq!(
            gate.poll_start(3),
            StartPermit::Wait,
            "its request postdates those reports"
        );
        report_at(&gate, &[0, 1], DUEL_HOLD);
        assert_eq!(gate.poll_start(3), StartPermit::Granted);

        report_at(&gate, &[2, 3], DUEL_PEN);
        report_at(&gate, &[2, 3], DUEL_HOLD);
        gate.mark_parked(2);
        gate.mark_parked(3);
        assert!(
            !gate.released(),
            "no slot has reported since the last pair parked"
        );
        report_at(&gate, &[0, 1, 2], DUEL_HOLD);
        assert!(!gate.released(), "slot 3 has not reported since");
        report_at(&gate, &[3], DUEL_HOLD);
        assert!(gate.released(), "staging is over");
        report_at(&gate, &[0, 1, 2, 3], DUEL_LOBBY);
        assert_eq!(gate.failure(), None, "released slots may enter the lobby");
    }

    /// R5: the frontend Started on each slot's latest clear bit, which could
    /// predate the Start by any amount (a stalled slot thread while the
    /// server moved the player). A grant now needs a report made after the
    /// request, so a stalled thread holds the Start and its report on
    /// resuming decides it.
    #[test]
    fn a_stale_clear_report_cannot_grant_a_start_while_its_slot_thread_is_stalled() {
        for resumed_at in [DUEL_NEAR_ZONE, DUEL_HOLD] {
            let gate = duel_gate_with_pair1_admitted();
            let live = AtomicBool::new(true);
            let (resume, stalled) = std::sync::mpsc::channel::<()>();
            std::thread::scope(|scope| {
                let slot0 = scope.spawn({
                    let gate = &gate;
                    move || {
                        stalled.recv().expect("resume slot 0");
                        gate.observe(0, duel_tile(resumed_at));
                    }
                });
                let slot1 = scope.spawn(|| {
                    while live.load(Ordering::Acquire) {
                        gate.observe(1, duel_tile(DUEL_HOLD));
                        std::thread::yield_now();
                    }
                });
                for _ in 0..200 {
                    assert_eq!(
                        gate.poll_start(2),
                        StartPermit::Wait,
                        "slot 0's clear reading predates the request while its thread is stalled"
                    );
                    std::thread::sleep(Duration::from_micros(200));
                }
                resume.send(()).expect("slot 0 is waiting");
                slot0.join().expect("slot 0 reports");
                let permit = decided_start(&gate, 2);
                live.store(false, Ordering::Release);
                slot1.join().expect("slot 1 reports");
                if resumed_at == DUEL_HOLD {
                    assert_eq!(permit, StartPermit::Granted, "a fresh clear report grants");
                } else {
                    assert_slot0_breach(permit);
                }
            });
        }
    }

    /// R5: admission checked the breach, then scanned independent per-slot
    /// bits, so a breach recorded between the two still Started the pair. A
    /// breach and a grant now share one lock, and the grant needs a report
    /// after the request: a breach reported concurrently with a Start request
    /// is never granted, whichever takes the lock first.
    #[test]
    fn a_breach_reported_concurrently_with_a_start_request_is_never_granted() {
        let gate = duel_gate_with_pair1_admitted();
        assert_eq!(gate.poll_start(2), StartPermit::Wait, "request first");
        report_at(&gate, &[0], DUEL_NEAR_ZONE);
        report_at(&gate, &[1], DUEL_HOLD);
        assert_slot0_breach(gate.poll_start(2));
        assert_slot0_breach(gate.poll_start(3));

        for _ in 0..500 {
            let gate = duel_gate_with_pair1_admitted();
            let barrier = std::sync::Barrier::new(3);
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    barrier.wait();
                    gate.observe(0, duel_tile(DUEL_NEAR_ZONE));
                });
                scope.spawn(|| {
                    barrier.wait();
                    for _ in 0..20 {
                        gate.observe(1, duel_tile(DUEL_HOLD));
                    }
                });
                barrier.wait();
                assert_slot0_breach(decided_start(&gate, 2));
            });
        }
    }

    /// R5: the fence lapsed once every slot had entered a pen, so a parked
    /// slot's late report from near the arena recorded nothing. It now stays
    /// up after every pair has entered a pen, and comes down only after the
    /// last pair has parked and every slot has reported clear since: a late
    /// in-zone report then is a breach, not a release.
    #[test]
    fn the_fence_stays_up_after_every_pair_has_entered_a_pen() {
        for last_pair_parked in [false, true] {
            let gate = duel_gate_with_pair1_admitted();
            assert_eq!(gate.poll_start(2), StartPermit::Wait);
            assert_eq!(gate.poll_start(3), StartPermit::Wait);
            report_at(&gate, &[0, 1], DUEL_HOLD);
            assert_eq!(gate.poll_start(2), StartPermit::Granted);
            assert_eq!(gate.poll_start(3), StartPermit::Granted);
            report_at(&gate, &[2, 3], DUEL_PEN);
            if last_pair_parked {
                report_at(&gate, &[2, 3], DUEL_HOLD);
                gate.mark_parked(2);
                gate.mark_parked(3);
                report_at(&gate, &[1, 2, 3], DUEL_HOLD);
                assert!(
                    !gate.released(),
                    "slot 0 has not reported since the last pair parked"
                );
            }
            report_at(&gate, &[0], DUEL_NEAR_ZONE);
            let failure = gate
                .failure()
                .expect("near the arena after every pair has entered a pen is a breach");
            assert_eq!(failure.slot, 0);
            assert!(
                failure.message.contains("clearance breach"),
                "{}",
                failure.message
            );
            report_at(&gate, &[0], DUEL_HOLD);
            assert!(!gate.released(), "a breach never lowers the fence");
        }
    }

    /// A slot whose client thread exits can no longer report, so while the
    /// fence is up its exit fails the fleet instead of stalling every later
    /// Start; once staging is over (teardown), it does not.
    #[test]
    fn a_slot_thread_exit_fails_the_fleet_only_while_the_fence_is_up() {
        let gate = duel_gate_with_pair1_admitted();
        assert_eq!(gate.poll_start(2), StartPermit::Wait);
        gate.retire(0);
        let StartPermit::Refused(failure) = gate.poll_start(2) else {
            panic!("a retired slot must refuse the pending Start");
        };
        assert_eq!(failure.slot, 0);
        assert!(
            failure.message.contains("client thread exited"),
            "{}",
            failure.message
        );

        let gate = duel_gate_with_pair1_admitted();
        for slot in [2, 3] {
            assert_eq!(gate.poll_start(slot), StartPermit::Wait);
            report_at(&gate, &[0, 1], DUEL_HOLD);
            assert_eq!(gate.poll_start(slot), StartPermit::Granted);
        }
        report_at(&gate, &[2, 3], DUEL_HOLD);
        gate.mark_parked(2);
        gate.mark_parked(3);
        report_at(&gate, &[0, 1, 2, 3], DUEL_HOLD);
        assert!(gate.released());
        for slot in 0..4 {
            gate.retire(slot);
        }
        assert_eq!(
            gate.failure(),
            None,
            "teardown after staging is not a failure"
        );
    }

    fn duel_client_at(tile: WorldTile) -> Client {
        let mut client = Client::new(client::client::ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: false,
        });
        client.ingame = true;
        client.set_cheat_admission(client::CheatAdmission::Granted);
        client.scene_state = 2;
        client.map_build_base_x = (tile.x >> 6) << 6;
        client.map_build_base_z = (tile.z >> 6) << 6;
        client.minusedlevel = tile.level;
        client.local_player = Some(ClientPlayer::at(
            tile.x - client.map_build_base_x,
            tile.z - client.map_build_base_z,
        ));
        for prot in [
            ServerProt::PLAYER_INFO,
            ServerProt::REBUILD_NORMAL,
            ServerProt::UPDATE_STAT,
        ] {
            client.bump_gens(prot);
        }
        client
    }

    fn set_duel_tile(client: &mut Client, tile: WorldTile) {
        client.map_build_base_x = (tile.x >> 6) << 6;
        client.map_build_base_z = (tile.z >> 6) << 6;
        client.minusedlevel = tile.level;
        client.local_player = Some(ClientPlayer::at(
            tile.x - client.map_build_base_x,
            tile.z - client.map_build_base_z,
        ));
        client.scene_state = 2;
        client.bump_gens(ServerProt::REBUILD_NORMAL);
        client.bump_gens(ServerProt::PLAYER_INFO);
    }

    fn out_contains(client: &Client, needle: &str) -> bool {
        let bytes = &client.out.data()[..client.out.pos];
        bytes
            .windows(needle.len())
            .any(|window| window == needle.as_bytes())
    }

    fn hold_tele_cheat() -> String {
        tele_args(DUEL_HOLD.level, DUEL_HOLD.x, DUEL_HOLD.z)
    }

    /// Slot 0 of a four-slot duel fleet from its completed-duel watch on,
    /// driven through `seed_frame` like `client_frame`, scene-settle gate off.
    fn duel_slot0_after_start(gate: &Arc<DuelPairGate>) -> Seed {
        let mut scenario = scenario::get("duel_arena").expect("duel scenario");
        qualify_duel_arena_fleet(&mut scenario, 4, 0, Some(gate)).expect("even fleet");
        let watch = scenario
            .steps
            .iter()
            .position(|step| step.name == "watch this slot finish a duel with another fleet member")
            .expect("completed-duel watch");
        scenario.steps.drain(..watch);
        scenario.seed.mainland = false;
        scenario.settings.require_mainland_base = false;
        let mut runner = scenario::ScenarioRunner::with_world(scenario, None);
        runner.set_scene_settle(Duration::ZERO);
        Seed {
            runner,
            started: false,
            step_names: Vec::new(),
            last_step: None,
            duel: Some(DuelSlot {
                gate: Arc::clone(gate),
                slot: 0,
            }),
        }
    }

    const DUEL_PEN: WorldTile = WorldTile {
        x: 3340,
        z: 3250,
        level: 0,
    };

    /// Drive slot 0 from the lobby through its duel back to the hold, where
    /// the keep-parked Repeat parks it. Pair 0's other member is reported and
    /// parked directly.
    fn park_duel_slot0(gate: &DuelPairGate, seed: &mut Seed, client: &mut Client) {
        seed_frame(seed, client, false);
        set_duel_tile(client, DUEL_PEN);
        seed_frame(seed, client, false);
        set_duel_tile(client, DUEL_LOBBY);
        seed_frame(seed, client, false);
        seed_frame(seed, client, false);
        set_duel_tile(client, DUEL_HOLD);
        seed_frame(seed, client, false);
        seed_frame(seed, client, false);
        gate.observe(1, duel_tile(DUEL_HOLD));
        gate.mark_parked(1);
    }

    /// `play_slots` runs `memory::client_frame` (this tick) then `slot_frame`
    /// (the frozen card). `ScenarioRunner` only `advance_step()`s when the
    /// completed-duel Await holds, so the parking send is not applied on that
    /// same tick: the already-fought slot is still in the challenge lobby when
    /// the card's `challengeTargets()` would run. Pair 1 must not be a lobby
    /// candidate then — it stays outside until this pair is parked at the hold.
    #[test]
    fn already_fought_pair_is_not_a_lobby_candidate_on_the_completed_duel_advance_frame() {
        let gate = DuelPairGate::new(4);
        for slot in 0..4 {
            gate.mark_holding(slot);
            gate.observe(slot, duel_tile(DUEL_HOLD));
        }

        let mut seed = duel_slot0_after_start(&gate);
        let mut client = duel_client_at(DUEL_LOBBY);

        seed_frame(&mut seed, &mut client, false);
        assert_eq!(
            seed.runner.status(),
            scenario::RunnerStatus::Running { step: 0, total: 3 },
            "seed falls into the completed-duel watch"
        );

        set_duel_tile(&mut client, DUEL_PEN);
        seed_frame(&mut seed, &mut client, false);
        assert!(
            !gate.admitted(2),
            "pen entry must not admit the next pair while this one can still return to the lobby"
        );

        let out_before = client.out.pos;
        set_duel_tile(&mut client, DUEL_LOBBY);
        seed_frame(&mut seed, &mut client, false);
        assert_eq!(
            seed.runner.status(),
            scenario::RunnerStatus::Running { step: 1, total: 3 },
            "completed-duel advances onto the parking tele"
        );
        assert_eq!(
            client.out.pos, out_before,
            "parking send is not applied on the advance tick"
        );
        assert!(
            !out_contains(&client, &hold_tele_cheat()),
            "the hold tele must not have gone out before slot_frame"
        );
        assert!(
            !gate.admitted(2),
            "a starting pair must not share the lobby with an already-fought slot on the advance frame"
        );

        seed_frame(&mut seed, &mut client, false);
        assert!(
            out_contains(&client, &hold_tele_cheat()),
            "the next tick sends the parking tele"
        );
        assert!(!gate.admitted(2), "in-flight tele is not parked");

        set_duel_tile(&mut client, DUEL_HOLD);
        seed_frame(&mut seed, &mut client, false);
        assert_eq!(
            seed.runner.status(),
            scenario::RunnerStatus::Running { step: 2, total: 3 },
            "arrival at the hold advances onto the keep-parked Repeat"
        );
        assert!(
            !gate.admitted(2),
            "arrival without the Repeat send has not latched parked"
        );

        seed_frame(&mut seed, &mut client, false);
        gate.observe(1, duel_tile(DUEL_HOLD));
        gate.mark_parked(1);
        assert!(
            gate.admitted(2),
            "both members parked at the hold admits the next pair"
        );
    }

    /// Parking used to be a sticky latch: the frozen card walks a parked slot
    /// back toward the arena, and the keep-parked Repeat's re-tele is neither
    /// acknowledged nor sent while the scene settles. If it does not apply, the
    /// already-fought slot can reach the lobby while a later admitted pair is
    /// still negotiating. Nearing the challenge area must be a named breach,
    /// seen on the slot's own frame even when the Repeat sends nothing, that
    /// refuses the admitted pair's Start permits and every later admission.
    /// Landing far from the arena (the Lumbridge spawn, seen live) is not a
    /// breach.
    #[test]
    fn a_parked_fighter_nearing_the_challenge_area_fails_the_fleet_before_a_later_pair_can_meet_it()
    {
        let gate = DuelPairGate::new(4);
        for slot in 0..4 {
            gate.mark_holding(slot);
            gate.observe(slot, duel_tile(DUEL_HOLD));
        }
        let mut seed = duel_slot0_after_start(&gate);
        let mut client = duel_client_at(DUEL_LOBBY);
        park_duel_slot0(&gate, &mut seed, &mut client);
        assert!(gate.admitted(2) && gate.admitted(3));
        gate.observe(2, duel_tile(DUEL_LOBBY));
        gate.observe(3, duel_tile(DUEL_LOBBY));

        // Pair 1 is admitted and has not entered a pen. The card walks slot 0
        // off the hold, or it lands at the Lumbridge spawn; the Repeat re-teles
        // it either way and neither is near the challenge area.
        const LUMBRIDGE: WorldTile = WorldTile {
            x: 3222,
            z: 3218,
            level: 0,
        };
        let walked = WorldTile {
            x: DUEL_HOLD.x + 22,
            z: DUEL_HOLD.z + 22,
            level: 0,
        };
        for tile in [walked, LUMBRIDGE] {
            set_duel_tile(&mut client, tile);
            let out_before = client.out.pos;
            seed_frame(&mut seed, &mut client, false);
            let sent = &client.out.data()[out_before..client.out.pos];
            assert!(
                sent.windows(hold_tele_cheat().len())
                    .any(|window| window == hold_tele_cheat().as_bytes()),
                "the Repeat corrects ({}, {})",
                tile.x,
                tile.z
            );
            assert_eq!(gate.failure(), None, "({}, {}) is clear", tile.x, tile.z);
            assert!(gate.admitted(2), "pair 1 stays admitted");
        }

        // That tele does not apply. A scene load arms the settle gate, so the
        // Repeat sends nothing while the card walks on toward the arena.
        seed.runner.set_scene_settle(Duration::from_secs(3600));
        client.scene_state = 1;
        seed_frame(&mut seed, &mut client, false);
        assert!(
            duel_zone_steps(DUEL_NEAR_ZONE.x, DUEL_NEAR_ZONE.z) > 0,
            "still outside the challenge area"
        );
        set_duel_tile(&mut client, DUEL_NEAR_ZONE);
        let out_before = client.out.pos;
        seed_frame(&mut seed, &mut client, false);
        assert_eq!(
            client.out.pos, out_before,
            "the settle-gated Repeat sent no correction"
        );
        assert_slot0_breach(gate.poll_start(2));
        assert_slot0_breach(gate.poll_start(3));
        assert!(
            !gate.admitted(2),
            "an admitted pair is withdrawn by the breach"
        );

        set_duel_tile(&mut client, DUEL_HOLD);
        seed_frame(&mut seed, &mut client, false);
        gate.observe(1, duel_tile(DUEL_HOLD));
        assert_slot0_breach(gate.poll_start(2));
    }

    #[test]
    fn seeded_idle_preserves_setup_and_ends_before_script_start() {
        let active = scenario::thiever_sustained_scenario();
        let idle = seeded_idle_scenario();
        let start = active
            .steps
            .iter()
            .position(|s| matches!(s.kind, scenario::StepKind::StartScript))
            .unwrap();
        assert_eq!(idle.steps.len(), start);
        assert_eq!(
            idle.steps.iter().map(|s| s.name).collect::<Vec<_>>(),
            active.steps[..start]
                .iter()
                .map(|s| s.name)
                .collect::<Vec<_>>()
        );
        assert!(matches!(
            idle.steps.last().unwrap().kind,
            scenario::StepKind::DrainDialogs { .. }
        ));
        assert!(matches!(
            idle.proof,
            scenario::Proof::ArrivedNear {
                x: 2661,
                z: 3306,
                level: 0,
                radius: 10
            }
        ));
        assert!(idle.settings.start_script.is_none());
    }

    #[test]
    fn prepare_seeded_idle_runs_seed_without_catalog() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::clear(&[
            "RS2B0T",
            "BOT_MEMORY_OUTPUT",
            "BOT_MEMORY_N",
            "BOT_MEMORY_WORKLOAD",
        ]);
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
        let _lock = ENV_LOCK.lock().unwrap();
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
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::clear(&["RS2B0T", "BOT_MEMORY_OUTPUT"]);
        let err = match Run::prepare(unit_config(1, Workload::Active), "unit") {
            Err(e) => e,
            Ok(_) => panic!("expected RS2B0T error"),
        };
        assert!(err.contains("RS2B0T"), "expected RS2B0T error, got {err}");
    }

    #[test]
    fn prepare_lifecycle_errors_without_rs2b0t() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::clear(&["RS2B0T", "BOT_MEMORY_OUTPUT"]);
        let err = match Run::prepare(unit_config(1, Workload::Lifecycle), "unit") {
            Err(e) => e,
            Ok(_) => panic!("expected RS2B0T error"),
        };
        assert!(err.contains("RS2B0T"), "expected RS2B0T error, got {err}");
    }

    #[test]
    fn require_live_benchmark_errors_without_live_env() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::clear(&["LIVE"]);
        let err = require_live_benchmark(crate::ProfileClass::Local).expect_err("LIVE unset");
        assert!(err.contains("LIVE=1"), "got {err}");
    }
}
