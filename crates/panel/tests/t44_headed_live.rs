//! Main-thread live fixture for the gated render implementation. Normal test
//! suites do not start it; it never activates the production panel binary.
use std::num::NonZeroUsize;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use api::identity::SlotKey;
use client::render::backend::BackendKind;
use client::render::budget::RenderBudgetLimits;
use frontend_core::surface::{
    configure_render_fixture, configure_render_fixture_budgets, configure_render_fixture_control,
    ProcessFixture, RenderTestControl, RenderTestEvent, RenderTestPoint,
};

#[cfg(feature = "memory-profile")]
#[global_allocator]
static ALLOCATOR: frontend_core::process::BenchmarkAllocator =
    frontend_core::process::BENCHMARK_ALLOCATOR;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DrawingBudgetInputs {
    capture: usize,
    nav_vertices: usize,
    nav_quads: usize,
    nav_edges: usize,
    nav_input: usize,
    head_models: usize,
    head_misc: usize,
    temp_models: usize,
    shared_models: usize,
    shared_sprites: usize,
    vertex: usize,
    context_cpu: usize,
    context_gpu: usize,
}

impl DrawingBudgetInputs {
    fn limits(self) -> RenderBudgetLimits {
        RenderBudgetLimits::new(
            self.capture,
            self.nav_vertices,
            self.nav_quads,
            self.nav_edges,
            self.nav_input,
            self.head_models,
            self.head_misc,
            self.temp_models,
            self.shared_models,
            self.shared_sprites,
            self.vertex,
            self.context_cpu,
            self.context_gpu,
        )
    }
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|error| format!("{name}: {error}"))
}

fn number(name: &str, value: &str, allow_zero: bool) -> Result<u64, String> {
    let value = value
        .parse::<u64>()
        .map_err(|error| format!("{name}: {error}"))?;
    if value == 0 && !allow_zero {
        return Err(format!("{name} must be positive"));
    }
    Ok(value)
}

fn positive(name: &str) -> Result<u64, String> {
    number(name, &required(name)?, false)
}

fn optional_pair(
    first: &str,
    second: &str,
    first_allows_zero: bool,
) -> Result<Option<(u64, u64)>, String> {
    match (std::env::var(first), std::env::var(second)) {
        (Err(std::env::VarError::NotPresent), Err(std::env::VarError::NotPresent)) => Ok(None),
        (Ok(a), Ok(b)) => Ok(Some((
            number(first, &a, first_allows_zero)?,
            number(second, &b, false)?,
        ))),
        _ => Err(format!("supply both {first} and {second}, or neither")),
    }
}

struct HeldLease {
    slot: u64,
    duration: Duration,
    used: AtomicBool,
    active: AtomicBool,
    finished: AtomicBool,
    gpu: AtomicBool,
    ui_before: AtomicU64,
    ui_after: AtomicU64,
    peer_slot: AtomicU64,
    peer_first: AtomicI32,
    peer_last: AtomicI32,
}

struct LiveControl {
    held: Option<HeldLease>,
    loading: Option<(Duration, Duration)>,
    first_scene: OnceLock<Instant>,
    loading_paints: AtomicU64,
    post_loading_paints: AtomicU64,
}

impl LiveControl {
    fn from_env() -> Result<Self, String> {
        let held =
            optional_pair("T44_HELD_SLOT", "T44_HELD_LEASE_MS", false)?.map(|(slot, millis)| {
                HeldLease {
                    slot,
                    duration: Duration::from_millis(millis),
                    used: AtomicBool::new(false),
                    active: AtomicBool::new(false),
                    finished: AtomicBool::new(false),
                    gpu: AtomicBool::new(false),
                    ui_before: AtomicU64::new(0),
                    ui_after: AtomicU64::new(0),
                    peer_slot: AtomicU64::new(0),
                    peer_first: AtomicI32::new(i32::MIN),
                    peer_last: AtomicI32::new(i32::MIN),
                }
            });
        let loading = optional_pair("T44_LOADING_AFTER_MS", "T44_LOADING_FOR_MS", true)?.map(
            |(after, duration)| {
                (
                    Duration::from_millis(after),
                    Duration::from_millis(duration),
                )
            },
        );
        if loading.is_some_and(|(after, duration)| after.checked_add(duration).is_none()) {
            return Err("the loading fixture interval overflows Duration".into());
        }
        Ok(Self {
            held,
            loading,
            first_scene: OnceLock::new(),
            loading_paints: AtomicU64::new(0),
            post_loading_paints: AtomicU64::new(0),
        })
    }

    fn validate(&self) -> Result<(), String> {
        let mut passed = true;
        if let Some(held) = &self.held {
            let before = held.ui_before.load(Ordering::Acquire);
            let after = held.ui_after.load(Ordering::Acquire);
            let first = held.peer_first.load(Ordering::Acquire);
            let last = held.peer_last.load(Ordering::Acquire);
            let peer_advanced = first != i32::MIN && last.wrapping_sub(first) > 0;
            let valid = held.used.load(Ordering::Acquire)
                && held.finished.load(Ordering::Acquire)
                && held.gpu.load(Ordering::Acquire)
                && after.checked_sub(before).is_some_and(|paints| paints > 0)
                && peer_advanced;
            passed &= valid;
            eprintln!(
                "t44_headed_held_lease={}",
                serde_json::json!({
                    "slot": held.slot,
                    "hold_ms": held.duration.as_millis(),
                    "used": held.used.load(Ordering::Acquire),
                    "finished": held.finished.load(Ordering::Acquire),
                    "actual_gpu": held.gpu.load(Ordering::Acquire),
                    "submitted_ui_before": before,
                    "submitted_ui_after": after,
                    "peer_slot": held.peer_slot.load(Ordering::Acquire),
                    "peer_cycle_before": first,
                    "peer_cycle_after": last,
                    "passed": valid,
                })
            );
        }
        if let Some((after, duration)) = self.loading {
            let loading = self.loading_paints.load(Ordering::Acquire);
            let resumed = self.post_loading_paints.load(Ordering::Acquire);
            let valid = loading > 0 && resumed > 0;
            passed &= valid;
            eprintln!(
                "t44_headed_loading_input={}",
                serde_json::json!({
                    "paint_only": true,
                    "after_ms": after.as_millis(),
                    "duration_ms": duration.as_millis(),
                    "loading_paints": loading,
                    "post_loading_paints": resumed,
                    "passed": valid,
                })
            );
        }
        if passed {
            Ok(())
        } else {
            Err("headed render fixture did not exercise its required progress".into())
        }
    }
}

impl RenderTestControl for LiveControl {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn loading_paint(&self) -> bool {
        let (Some((after, duration)), Some(started)) = (self.loading, self.first_scene.get())
        else {
            return false;
        };
        let elapsed = started.elapsed();
        if elapsed < after {
            return false;
        }
        if elapsed - after < duration {
            self.loading_paints.fetch_add(1, Ordering::Relaxed);
            true
        } else {
            self.post_loading_paints.fetch_add(1, Ordering::Relaxed);
            false
        }
    }

    fn client_cycle(
        &self,
        slot: SlotKey,
        backend: BackendKind,
        cycle: i32,
        complete_game_scene: bool,
    ) {
        if complete_game_scene {
            self.first_scene.get_or_init(Instant::now);
        }
        let Some(held) = &self.held else {
            return;
        };
        let slot = slot.incarnation().get();
        if slot == held.slot {
            held.gpu
                .store(backend == BackendKind::Gpu, Ordering::Release);
            return;
        }
        if !complete_game_scene {
            return;
        }
        let peer = held
            .peer_slot
            .compare_exchange(0, slot, Ordering::AcqRel, Ordering::Acquire)
            .unwrap_or_else(|existing| existing);
        if peer != 0 && peer != slot {
            return;
        }
        if !held.active.load(Ordering::Acquire) {
            return;
        }
        let _ =
            held.peer_first
                .compare_exchange(i32::MIN, cycle, Ordering::AcqRel, Ordering::Acquire);
        held.peer_last.store(cycle, Ordering::Release);
    }

    fn at(&self, event: RenderTestEvent) {
        if event.point != RenderTestPoint::BeforeLeaseRelease || !event.owns_lease {
            return;
        }
        let Some(held) = &self.held else {
            return;
        };
        if event.head.slot().incarnation().get() != held.slot
            || held.peer_slot.load(Ordering::Acquire) == 0
            || held.used.swap(true, Ordering::AcqRel)
        {
            return;
        }
        let Some(deadline) = Instant::now().checked_add(held.duration) else {
            return;
        };
        held.ui_before.store(
            panel::window::test_hooks::submitted_paints(),
            Ordering::Release,
        );
        held.active.store(true, Ordering::Release);
        eprintln!(
            "t44_headed_hold_begin={}",
            serde_json::json!({
                "slot": held.slot, "head": format!("{:?}", event.head),
                "reservation": format!("{:?}", event.reservation),
                "submitted_ui": held.ui_before.load(Ordering::Acquire),
            })
        );
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            std::thread::park_timeout(remaining);
        }
        held.ui_after.store(
            panel::window::test_hooks::submitted_paints(),
            Ordering::Release,
        );
        held.active.store(false, Ordering::Release);
        held.finished.store(true, Ordering::Release);
        eprintln!("t44_headed_hold_end slot={}", held.slot);
    }
}

fn run_fixture() -> Result<(), String> {
    if required("LIVE")? != "1" {
        return Err("T44_HEADED_LIVE requires LIVE=1".into());
    }
    frontend_core::process::bootstrap().map_err(|error| error.to_string())?;
    let ttl = positive("M3_GRANT_TTL_MS")?;
    let quantum = positive("M3_SCENE_QUANTUM_MS")?;
    let budgets: DrawingBudgetInputs =
        serde_json::from_str(&required("T44_RENDER_BUDGET_LIMITS_JSON")?)
            .map_err(|error| format!("T44_RENDER_BUDGET_LIMITS_JSON: {error}"))?;
    let retained = usize::try_from(positive("T44_RETAINED_PRESENT_CAP")?)
        .map_err(|error| error.to_string())?;
    let retained =
        NonZeroUsize::new(retained).ok_or("the retained-present cap must be positive")?;
    positive("T44_CAPTURE_METADATA_BYTES")?;
    positive("T44_CAPTURE_READBACK_TIMEOUT_MS")?;
    positive("T44_RECORD_FINISH_TIMEOUT_MS")?;
    let redraw = match required("T44_HEADED_REDRAW")?.as_str() {
        "wait" => panel::window::RedrawMode::Wait,
        "normal" => panel::runner_config().redraw,
        _ => return Err("T44_HEADED_REDRAW must be wait or normal".into()),
    };
    let control = Arc::new(LiveControl::from_env()?);
    eprintln!(
        "t44_headed_fixture_layout={}",
        serde_json::json!({
            "fixture_owned_control_arc_allocation_bytes": std::mem::size_of::<LiveControl>()
                + 2 * std::mem::size_of::<usize>(),
            "held_lease_inline_bytes": std::mem::size_of::<HeldLease>(),
            "submitted_ui_counter_static_bytes": std::mem::size_of::<AtomicU64>(),
            "scope": "diagnostic fixture only; excluded from renderer owner totals",
        })
    );
    configure_render_fixture(ProcessFixture {
        grant_ttl: Duration::from_millis(ttl),
        quantum: Duration::from_millis(quantum),
    })
    .map_err(|error| error.to_string())?;
    configure_render_fixture_budgets(budgets.limits()).map_err(|error| error.to_string())?;
    client::render::backend::configure_retained_present_budget(retained)
        .map_err(|error| error.to_string())?;
    configure_render_fixture_control(control.clone()).map_err(|error| error.to_string())?;
    let args = panel::parse_args(
        std::env::args().skip(1),
        std::env::var("BOT_LIVE").ok().as_deref(),
    )
    .map_err(|(_, message)| message)?;
    frontend_core::process::raise_fd_limit();
    let result = panel::run_panel_render_fixture(args, redraw).map_err(|error| error.to_string());
    let evidence = control.validate();
    let shutdown =
        frontend_core::process::shutdown_background_work().map_err(|error| error.to_string());
    result?;
    evidence?;
    shutdown
}

fn main() -> ExitCode {
    if std::env::var("T44_HEADED_LIVE").as_deref() != Ok("1") {
        println!("t44_headed_live not selected; explicit live fixture inputs are required");
        return ExitCode::SUCCESS;
    }
    match run_fixture() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("t44_headed_live FAILED: {error}");
            ExitCode::FAILURE
        }
    }
}
