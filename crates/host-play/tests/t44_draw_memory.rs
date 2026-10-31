//! Drawing-only allocation evidence. M3 timing never installs this allocator.
#![cfg(feature = "draw-memory-profile")]

use client::render::backend::{cpu::CpuBackend, RenderBackend};
use host_play::memory::{native_allocator_sample, rust_allocator_counts, CountingAllocator};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[test]
fn drawing_memory_target_is_not_the_timing_allocator() {
    const CHILD: &str = "T44_ISOLATED_ALLOCATOR";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "drawing_memory_target_is_not_the_timing_allocator",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let before = rust_allocator_counts();
    let allocation = std::hint::black_box(Box::new([0x5a_u8; 23]));
    let owned = rust_allocator_counts();
    drop(allocation);
    let freed = rust_allocator_counts();
    assert_eq!(owned.0 - before.0, 1);
    assert_eq!(owned.1 - before.1, 23);
    assert_eq!(owned.2 - before.2, 23);
    assert_eq!(freed.2, before.2);
}

#[test]
fn cpu_retained_storage_allocates_once_after_warmup() {
    const CHILD: &str = "T44_ISOLATED_COPY";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "cpu_retained_storage_allocates_once_after_warmup",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }

    let pixels = vec![0x0034_5678; 512 * 334];
    let mut backend = CpuBackend::default();
    let before = rust_allocator_counts();
    let pointer = backend.profile_clean_copy(&pixels).as_ptr();
    let initialized = rust_allocator_counts();
    assert_eq!(initialized.0 - before.0, 1);
    assert_eq!(initialized.1 - before.1, 684_032);
    assert_eq!(initialized.2 - before.2, 684_032);
    assert_eq!(backend.retained_storage_allocations(), 1);
    assert_eq!(backend.retained_storage_bytes(), 684_032);

    let before_warm = rust_allocator_counts();
    for _ in 0..8 {
        let clean = backend.profile_clean_copy(&pixels);
        assert_eq!(clean.as_ptr(), pointer);
        assert_eq!(clean, pixels.as_slice());
    }
    assert_eq!(rust_allocator_counts(), before_warm);
    assert_eq!(backend.retained_storage_allocations(), 1);
    let before_drop = rust_allocator_counts();
    drop(backend);
    let after_drop = rust_allocator_counts();
    assert_eq!(before_drop.2 - after_drop.2, 684_032);
}
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use api::interact;
use api::snapshot::GameSnapshot;
use client::client::Client;
use client::render::backend::{BackendKind, FrameOutput, SceneProgress};
use client::render::budget::RenderBytes;
use host::render_head::test_hooks::{RenderTestControl, RenderTestEvent};
use host::{render_head, FrameBuf, Pump, SlotInput};
use host_play::walk_map::{ActionKind, FocusToken, MapContext, MapModel};
use host_play::{
    current_resident_bytes, mint_live_names, run_with_template, Play, ProfileOptions,
    SharedClientTemplate, SlotArm, WalkArm, WalkArms,
};
use nav_planner::bank_fetch::BankRows;
use nav_planner::map::identity::Digest;
use nav_planner::router::{FindOptions, Leg, Route};
use nav_planner::tile::Tile;
use nav_planner::transport::DoorDir;
use nav_planner::WorldState;
use serde_json::{json, Value};
use vault::{Profile, ProfileSettings, RasterMode};

#[derive(Clone, Debug)]
struct RegionClientEvent {
    tick: u32,
    tile: Option<(i32, i32, i32)>,
    scene_state: i32,
    base: (i32, i32),
    centre_zone: (i32, i32),
    minimap_flag: (i32, i32),
    main_overlay_id: i32,
    main_modal_id: i32,
    phase: &'static str,
    reasons: Vec<&'static str>,
    expected_generation: u64,
}

#[derive(Clone, Debug)]
struct RegionFrameEvidence {
    event_ids: Vec<usize>,
    generation: u64,
    kind: &'static str,
    width: Option<i32>,
    height: Option<i32>,
    pixel_count: Option<usize>,
    pixel_hash_fnv1a64: Option<u64>,
    splash_band_hash_fnv1a64: Option<u64>,
    minimap_band_hash_fnv1a64: Option<u64>,
    scene_progress: Option<String>,
    has_valid_scene: Option<bool>,
    scene_consumes: Option<u64>,
}

#[derive(Default)]
struct RegionTrace {
    client_events: Vec<RegionClientEvent>,
    frames: Vec<RegionFrameEvidence>,
}

fn pixel_hash(pixels: &[i32]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for pixel in pixels {
        hash ^= *pixel as u32 as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn pixel_region_hash(
    pixels: &[i32],
    width: usize,
    height: usize,
    x: std::ops::Range<usize>,
    y: std::ops::Range<usize>,
) -> Option<u64> {
    if x.start >= x.end || y.start >= y.end || x.end > width || y.end > height {
        return None;
    }
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for row in y {
        for column in x.clone() {
            let pixel = *pixels.get(row.checked_mul(width)?.checked_add(column)?)?;
            hash ^= pixel as u32 as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    Some(hash)
}

fn region_trace_json(trace: &RegionTrace) -> Value {
    json!({
        "client_events": trace.client_events.iter().enumerate().map(|(id, event)| json!({
            "id": id,
            "tick": event.tick,
            "tile": event.tile,
            "scene_state_before_paint": event.scene_state,
            "map_build_base": [event.base.0, event.base.1],
            "centre_zone": [event.centre_zone.0, event.centre_zone.1],
            "minimap_flag": [event.minimap_flag.0, event.minimap_flag.1],
            "main_overlay_id": event.main_overlay_id,
            "main_modal_id": event.main_modal_id,
            "phase": event.phase,
            "reasons": event.reasons,
            "expected_frame_generation": event.expected_generation,
        })).collect::<Vec<_>>(),
        "paint_frames": trace.frames.iter().map(|frame| json!({
            "events": frame.event_ids,
            "generation": frame.generation,
            "kind": frame.kind,
            "width": frame.width,
            "height": frame.height,
            "pixel_count": frame.pixel_count,
            "pixel_hash_fnv1a64": frame.pixel_hash_fnv1a64,
            "loading_splash_band_hash_fnv1a64": frame.splash_band_hash_fnv1a64,
            "minimap_band_hash_fnv1a64": frame.minimap_band_hash_fnv1a64,
            "scene_progress": frame.scene_progress,
            "has_valid_scene": frame.has_valid_scene,
            "scene_consumes": frame.scene_consumes,
        })).collect::<Vec<_>>(),
    })
}

#[derive(Default)]
struct OutputProbe {
    latest: Mutex<Option<OutputObservation>>,
    region_trace: Option<Arc<Mutex<RegionTrace>>>,
}

impl OutputProbe {
    fn for_region_trace(region_trace: Arc<Mutex<RegionTrace>>) -> Self {
        Self {
            latest: Mutex::new(None),
            region_trace: Some(region_trace),
        }
    }

    fn observe(&self, mailbox: &FrameBuf, output: &FrameOutput) {
        let (kind, width, height, pixels) = match output {
            FrameOutput::PixMap(pixmap) => (
                "pixmap",
                Some(pixmap.width),
                Some(pixmap.height),
                Some(pixmap.pixels.len()),
            ),
            FrameOutput::Texture(texture) => (
                "texture",
                Some(texture.width() as i32),
                Some(texture.height() as i32),
                None,
            ),
            FrameOutput::Refused(_) => ("refused", None, None, None),
        };
        let fault = output
            .render_fault()
            .or_else(|| mailbox.render_fault())
            .map(|_| "render_fault");
        let generation = mailbox.generation();
        let sample = OutputObservation {
            kind,
            width,
            height,
            pixels,
            fresh: output.is_fresh(),
            fault,
            generation,
        };
        *self.latest.lock().unwrap() = Some(sample);

        let Some(region_trace) = &self.region_trace else {
            return;
        };
        let event_ids: Vec<_> = {
            let trace = region_trace.lock().unwrap();
            trace
                .client_events
                .iter()
                .enumerate()
                .filter_map(|(id, event)| (event.expected_generation == generation).then_some(id))
                .collect()
        };
        if event_ids.is_empty() || generation == 0 {
            return;
        }
        let (pixel_hash_fnv1a64, splash_band_hash_fnv1a64, minimap_band_hash_fnv1a64) = match output
        {
            FrameOutput::PixMap(pixmap) => {
                let width = pixmap.width.max(0) as usize;
                let height = pixmap.height.max(0) as usize;
                (
                    Some(pixel_hash(&pixmap.pixels)),
                    pixel_region_hash(&pixmap.pixels, width, height, 120..400, 128..180),
                    pixel_region_hash(&pixmap.pixels, width, height, 516..765, 0..176),
                )
            }
            _ => (None, None, None),
        };
        let scene_stats = mailbox.fair_stats().filter(|stats| {
            mailbox
                .head()
                .is_some_and(|head| head_key(head) == head_key(stats.identity()))
        });
        let (scene_progress, has_valid_scene, scene_consumes) =
            scene_stats.map_or((None, None, None), |stats| {
                (
                    Some(format!("{:?}", stats.scene_progress())),
                    Some(stats.has_valid_scene()),
                    Some(stats.scene_consumes()),
                )
            });
        let mut trace = region_trace.lock().unwrap();
        if trace.frames.len() < 1024 {
            trace.frames.push(RegionFrameEvidence {
                event_ids,
                generation,
                kind,
                width,
                height,
                pixel_count: pixels,
                pixel_hash_fnv1a64,
                splash_band_hash_fnv1a64,
                minimap_band_hash_fnv1a64,
                scene_progress,
                has_valid_scene,
                scene_consumes,
            });
        }
    }

    fn latest(&self) -> Option<OutputObservation> {
        *self.latest.lock().unwrap()
    }
}

const CONSUMER_PERIOD: Duration = Duration::from_millis(16);
const SAMPLE_PERIOD: Duration = Duration::from_secs(1);
const OWNER_SAMPLE_TIMEOUT: Duration = Duration::from_secs(15);

static EPOCH: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn required(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("{key} is required"))
}

#[derive(Clone, Copy, serde::Deserialize)]
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

fn configure_drawing_fixture() {
    let grant_ttl_ms = required("M3_GRANT_TTL_MS")
        .parse::<u64>()
        .expect("M3_GRANT_TTL_MS is a positive fixture duration");
    let quantum_ms = required("M3_SCENE_QUANTUM_MS")
        .parse::<u64>()
        .expect("M3_SCENE_QUANTUM_MS is a positive fixture duration");
    assert!(grant_ttl_ms > 0 && quantum_ms > 0);
    let budgets: DrawingBudgetInputs =
        serde_json::from_str(&required("T44_RENDER_BUDGET_LIMITS_JSON"))
            .expect("all thirteen explicit fixture byte ceilings are required");
    render_head::test_hooks::configure_process(render_head::test_hooks::ProcessFixture {
        grant_ttl: Duration::from_millis(grant_ttl_ms),
        quantum: Duration::from_millis(quantum_ms),
    })
    .expect("configure drawing fixture before creating any renderer");
    render_head::test_hooks::configure_render_budgets(
        client::render::budget::RenderBudgetLimits::new(
            budgets.capture,
            budgets.nav_vertices,
            budgets.nav_quads,
            budgets.nav_edges,
            budgets.nav_input,
            budgets.head_models,
            budgets.head_misc,
            budgets.temp_models,
            budgets.shared_models,
            budgets.shared_sprites,
            budgets.vertex,
            budgets.context_cpu,
            budgets.context_gpu,
        ),
    )
    .expect("configure census-derived fixture budgets before the first head");
}

#[derive(Clone)]
struct PaintControl {
    loading: Arc<AtomicBool>,
    loading_paints: Arc<AtomicU64>,
}

impl RenderTestControl for PaintControl {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn loading_paint(&self) -> bool {
        let loading = self.loading.load(Ordering::SeqCst);
        if loading {
            self.loading_paints.fetch_add(1, Ordering::Relaxed);
        }
        loading
    }

    fn at(&self, _event: RenderTestEvent) {}
}

#[derive(Clone, Copy)]
struct OutputObservation {
    kind: &'static str,
    width: Option<i32>,
    height: Option<i32>,
    pixels: Option<usize>,
    fresh: bool,
    fault: Option<&'static str>,
    generation: u64,
}

struct HeadProbe {
    name: String,
    input: Arc<SlotInput>,
    mailbox: Arc<FrameBuf>,
    control: PaintControl,
    output: Arc<OutputProbe>,
}
struct ConsumerGuard {
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}

impl ConsumerGuard {
    fn stop_and_join(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.join
            .take()
            .expect("drawing-memory consumer handle")
            .join()
            .expect("join drawing-memory frame consumer");
    }
}

impl Drop for ConsumerGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HeadKey {
    slot: u64,
    generation: u64,
}

fn head_key(identity: host::render::HeadIdentity) -> HeadKey {
    HeadKey {
        slot: identity.slot().incarnation().get(),
        generation: identity.generation().get(),
    }
}

#[derive(Clone)]
struct DrawingHeadObservation {
    name: String,
    status_present: bool,
    connected: bool,
    ingame: bool,
    scene_state: i32,
    stream_epoch: u32,
    login_started: Option<Instant>,
    head_identity: Option<HeadKey>,
    last_stats_identity: Option<HeadKey>,
    stats_identity: Option<HeadKey>,
    backend: Option<BackendKind>,
    frame_generation: u64,
    paint_delta: u64,
    output: Option<OutputObservation>,
    scene_consumes: u64,
    scene_consumes_delta: Option<u64>,
    has_valid_scene: bool,
    scene_progress: Option<SceneProgress>,
    retained_storage_allocations: u64,
    retained_storage_bytes: usize,
    owner_bytes: Option<RenderBytes>,
    owner_cpu_retained_bytes: Option<usize>,
    refusal: Option<String>,
    loading_paints: u64,
}

impl DrawingHeadObservation {
    fn json(&self) -> Value {
        let output = self.output.map(|sample| {
            json!({
                "kind": sample.kind,
                "width": sample.width,
                "height": sample.height,
                "pixel_count": sample.pixels,
                "fresh": sample.fresh,
                "render_fault": sample.fault,
                "mailbox_generation_at_take": sample.generation,
            })
        });
        json!({
            "head": self.name,
            "status": {
                "present": self.status_present,
                "connected": self.connected,
                "ingame": self.ingame,
                "scene_state": self.scene_state,
                "stream_epoch": self.stream_epoch,
                "login_started_elapsed_s": self.login_started.map(|at| {
                    at.duration_since(*EPOCH).as_secs_f64()
                }),
            },
            "mailbox": {
                "generation": self.frame_generation,
                "paint_delta": self.paint_delta,
                "attached_head": self.head_identity.map(|identity| json!({
                    "slot_incarnation": identity.slot,
                    "head_generation": identity.generation,
                })),
                "last_stats_identity": self.last_stats_identity.map(|identity| json!({
                    "slot_incarnation": identity.slot,
                    "head_generation": identity.generation,
                })),
                "active_stats_identity": self.stats_identity.map(|identity| json!({
                    "slot_incarnation": identity.slot,
                    "head_generation": identity.generation,
                })),
                "refusal": self.refusal,
            },
            "render": {
                "backend": self.backend.map(|backend| format!("{backend:?}")),
                "scene_consumes": self.scene_consumes,
                "scene_consumes_delta": self.scene_consumes_delta,
                "has_valid_scene": self.has_valid_scene,
                "scene_progress": self.scene_progress.map(|progress| format!("{progress:?}")),
                "retained_storage_allocations": self.retained_storage_allocations,
                "retained_storage_bytes": self.retained_storage_bytes,
                "owner_census": self.owner_bytes.map(render_bytes_json),
            },
            "frame_output": output,
            "loading_paint_calls": self.loading_paints,
        })
    }
}

const RETAINED_PRESENT_GPU_BYTES_PER_RESOURCE: usize = 1_539_180;

/// A phase records product counters, never counts inferred from registrations.
#[derive(Clone, Debug, serde::Serialize)]
struct GpuRetainedReceipt {
    phase: GpuRetainedPhase,
    expected_resources: usize,
    cap: usize,
    live_resources: usize,
    logical_bytes: usize,
    allocations_before: u64,
    allocations_after: u64,
    copies_before: u64,
    copies_after: u64,
    copied_bytes_before: u64,
    copied_bytes_after: u64,
    copy_encode_ns: u128,
    present_submit_ns: u128,
    heads: Vec<GpuRetainedHead>,
    overflow_refused: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
enum GpuRetainedPhase {
    Off,
    Registered,
    Held,
    Draining,
    Drained,
}

#[derive(Clone, Debug, serde::Serialize)]
struct GpuRetainedHead {
    slot: u64,
    generation: u64,
    connected: bool,
    gpu: bool,
    valid_scene: bool,
    fault: bool,
    cpu_clean_bytes: usize,
    paints_before: u64,
    paints_after: u64,
    scenes_before: u64,
    scenes_after: u64,
    held_token_before: Option<u64>,
    held_token_after: Option<u64>,
}

fn gpu_retained_verdict(receipt: &GpuRetainedReceipt) -> Result<(), String> {
    let active = matches!(
        receipt.phase,
        GpuRetainedPhase::Registered | GpuRetainedPhase::Held | GpuRetainedPhase::Draining
    );
    if receipt.cap == 0
        || receipt.expected_resources == 0
        || receipt.expected_resources > receipt.cap
        || receipt.heads.len() != receipt.expected_resources
    {
        return Err("missing heads or invalid retained resource cap".into());
    }
    let expected_live = if active {
        receipt.expected_resources
    } else {
        0
    };
    if receipt.live_resources != expected_live
        || receipt.logical_bytes != expected_live * RETAINED_PRESENT_GPU_BYTES_PER_RESOURCE
    {
        return Err("retained resource count or logical byte charge is incorrect".into());
    }
    let allocations = receipt
        .allocations_after
        .checked_sub(receipt.allocations_before)
        .ok_or("allocation counter regressed")?;
    let expected_allocations = if receipt.phase == GpuRetainedPhase::Registered {
        receipt.expected_resources as u64
    } else {
        0
    };
    if allocations != expected_allocations {
        return Err("retained allocation was leaked, repeated, or omitted".into());
    }
    let copies = receipt
        .copies_after
        .checked_sub(receipt.copies_before)
        .ok_or("copy counter regressed")?;
    let copied_bytes = receipt
        .copied_bytes_after
        .checked_sub(receipt.copied_bytes_before)
        .ok_or("copied-byte counter regressed")?;
    if copies.checked_mul(RETAINED_PRESENT_GPU_BYTES_PER_RESOURCE as u64) != Some(copied_bytes) {
        return Err("retained copy byte count is incorrect".into());
    }
    if receipt.phase == GpuRetainedPhase::Registered {
        if copies < receipt.expected_resources as u64 {
            return Err("a registered head did not produce a retained copy".into());
        }
    } else if copies != 0 {
        return Err("unregistered or held resource received a retained copy".into());
    }
    if active && receipt.expected_resources == receipt.cap && !receipt.overflow_refused {
        return Err("resource q+1 was not refused".into());
    }
    for (index, head) in receipt.heads.iter().enumerate() {
        if head.slot == 0
            || head.generation == 0
            || receipt.heads[..index]
                .iter()
                .any(|other| other.slot == head.slot)
        {
            return Err("missing or duplicate head identity".into());
        }
        if !head.connected || !head.gpu || !head.valid_scene || head.fault {
            return Err("stopped head, wrong backend, invalid scene, or device fault".into());
        }
        if head.cpu_clean_bytes != 0 {
            return Err("embedded GPU CPU helper owns a clean-scene copy".into());
        }
        if head.paints_after <= head.paints_before || head.scenes_after <= head.scenes_before {
            return Err("ordinary display or scene progress stopped".into());
        }
        if matches!(
            receipt.phase,
            GpuRetainedPhase::Held | GpuRetainedPhase::Draining
        ) && (head.held_token_before.is_none()
            || head.held_token_before != head.held_token_after)
        {
            return Err("held frame identity changed or disappeared".into());
        }
    }
    Ok(())
}

#[test]
fn gpu_retained_oracle_accepts_each_correct_phase() {
    for phase in [
        GpuRetainedPhase::Off,
        GpuRetainedPhase::Registered,
        GpuRetainedPhase::Held,
        GpuRetainedPhase::Draining,
        GpuRetainedPhase::Drained,
    ] {
        assert!(gpu_retained_verdict(&gpu_retained_fixture(phase)).is_ok());
    }
}

fn gpu_retained_fixture(phase: GpuRetainedPhase) -> GpuRetainedReceipt {
    let active = matches!(
        phase,
        GpuRetainedPhase::Registered | GpuRetainedPhase::Held | GpuRetainedPhase::Draining
    );
    let registered = phase == GpuRetainedPhase::Registered;
    GpuRetainedReceipt {
        phase,
        expected_resources: 1,
        cap: 1,
        live_resources: usize::from(active),
        logical_bytes: usize::from(active) * RETAINED_PRESENT_GPU_BYTES_PER_RESOURCE,
        allocations_before: 0,
        allocations_after: u64::from(registered),
        copies_before: 0,
        copies_after: u64::from(registered),
        copied_bytes_before: 0,
        copied_bytes_after: u64::from(registered) * RETAINED_PRESENT_GPU_BYTES_PER_RESOURCE as u64,
        copy_encode_ns: 1,
        present_submit_ns: 1,
        heads: vec![GpuRetainedHead {
            slot: 1,
            generation: 1,
            connected: true,
            gpu: true,
            valid_scene: true,
            fault: false,
            cpu_clean_bytes: 0,
            paints_before: 1,
            paints_after: 2,
            scenes_before: 1,
            scenes_after: 2,
            held_token_before: Some(1),
            held_token_after: Some(1),
        }],
        overflow_refused: true,
    }
}

#[test]
fn gpu_retained_oracle_rejects_each_named_failure() {
    type Mutation = fn(&mut GpuRetainedReceipt);
    let cases: &[(GpuRetainedPhase, Mutation, &str)] = &[
        (
            GpuRetainedPhase::Off,
            |r| r.live_resources = 1,
            "resource count",
        ),
        (
            GpuRetainedPhase::Registered,
            |r| r.logical_bytes -= 1,
            "logical byte",
        ),
        (
            GpuRetainedPhase::Registered,
            |r| r.allocations_after = 2,
            "allocation",
        ),
        (
            GpuRetainedPhase::Registered,
            |r| r.copies_after = 0,
            "byte count",
        ),
        (
            GpuRetainedPhase::Registered,
            |r| {
                r.copies_after = 0;
                r.copied_bytes_after = 0;
            },
            "did not produce",
        ),
        (
            GpuRetainedPhase::Off,
            |r| {
                r.copies_after = 1;
                r.copied_bytes_after = RETAINED_PRESENT_GPU_BYTES_PER_RESOURCE as u64;
            },
            "received a retained copy",
        ),
        (
            GpuRetainedPhase::Held,
            |r| r.heads[0].held_token_after = Some(2),
            "identity changed",
        ),
        (
            GpuRetainedPhase::Draining,
            |r| r.live_resources = 0,
            "resource count",
        ),
        (
            GpuRetainedPhase::Drained,
            |r| r.live_resources = 1,
            "resource count",
        ),
        (
            GpuRetainedPhase::Held,
            |r| r.overflow_refused = false,
            "q+1",
        ),
        (
            GpuRetainedPhase::Held,
            |r| r.heads[0].cpu_clean_bytes = 684_032,
            "CPU helper",
        ),
        (
            GpuRetainedPhase::Held,
            |r| r.heads[0].paints_after = 1,
            "progress stopped",
        ),
        (
            GpuRetainedPhase::Held,
            |r| r.heads[0].scenes_after = 1,
            "progress stopped",
        ),
        (
            GpuRetainedPhase::Held,
            |r| r.heads[0].connected = false,
            "stopped head",
        ),
        (
            GpuRetainedPhase::Held,
            |r| r.heads[0].fault = true,
            "device fault",
        ),
        (
            GpuRetainedPhase::Held,
            |r| r.heads[0].gpu = false,
            "wrong backend",
        ),
        (GpuRetainedPhase::Held, |r| r.heads.clear(), "missing heads"),
        (
            GpuRetainedPhase::Held,
            |r| r.allocations_before = 1,
            "counter regressed",
        ),
    ];
    for (phase, mutate, expected) in cases {
        let mut receipt = gpu_retained_fixture(*phase);
        mutate(&mut receipt);
        let error = gpu_retained_verdict(&receipt).unwrap_err();
        assert!(
            error.contains(expected),
            "{phase:?}: {error}, expected {expected}"
        );
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ProcessRenderCensus {
    shared_host_capture_storage_bytes: usize,
    media_process_storage_bytes: usize,
    retained_present_resource_count: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ProcessRenderReconciliation {
    head_local_capture_storage_bytes: usize,
    shared_host_capture_storage_bytes: usize,
    max_head_context_cpu_bytes: usize,
    media_process_storage_bytes: usize,
    retained_present_resource_count: usize,
}

impl ProcessRenderReconciliation {
    fn capture_storage_bytes(self) -> usize {
        self.head_local_capture_storage_bytes
            .saturating_add(self.shared_host_capture_storage_bytes)
    }

    fn context_cpu_bytes(self) -> usize {
        // Per-head Renderer rows already include shared Media storage.
        self.max_head_context_cpu_bytes
            .max(self.media_process_storage_bytes)
    }

    fn retained_present_logical_gpu_bytes(self) -> usize {
        self.retained_present_resource_count
            .saturating_mul(RETAINED_PRESENT_GPU_BYTES_PER_RESOURCE)
    }

    fn json(self) -> Value {
        json!({
            "head_local_capture_storage_bytes": self.head_local_capture_storage_bytes,
            "shared_host_capture_storage_bytes": self.shared_host_capture_storage_bytes,
            "reconciled_capture_storage_bytes": self.capture_storage_bytes(),
            "max_head_context_cpu_bytes": self.max_head_context_cpu_bytes,
            "media_process_storage_bytes": self.media_process_storage_bytes,
            "process_context_cpu_bytes": self.context_cpu_bytes(),
            "retained_present_resource_count": self.retained_present_resource_count,
            "retained_present_logical_gpu_bytes": self.retained_present_logical_gpu_bytes(),
        })
    }
}

fn reconcile_process_render(
    census: ProcessRenderCensus,
    heads: &[DrawingHeadObservation],
) -> ProcessRenderReconciliation {
    let head_local_capture_storage_bytes = heads
        .iter()
        .filter_map(|head| head.owner_bytes)
        .map(RenderBytes::capture)
        .fold(0usize, usize::saturating_add);
    let max_head_context_cpu_bytes = heads
        .iter()
        .filter_map(|head| head.owner_bytes)
        .map(RenderBytes::context_cpu)
        .fold(0usize, usize::max);
    ProcessRenderReconciliation {
        head_local_capture_storage_bytes,
        shared_host_capture_storage_bytes: census.shared_host_capture_storage_bytes,
        max_head_context_cpu_bytes,
        media_process_storage_bytes: census.media_process_storage_bytes,
        retained_present_resource_count: census.retained_present_resource_count,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DrawingPhase {
    Headless,
    Live,
    Freeze,
    Detached,
    Reattach,
}

fn drawing_memory_verdict(
    phase: DrawingPhase,
    expected_heads: usize,
    heads: &[DrawingHeadObservation],
    reference: Option<&[DrawingHeadObservation]>,
) -> Result<(), String> {
    let process = reconcile_process_render(ProcessRenderCensus::default(), heads);
    drawing_memory_verdict_with_process(phase, expected_heads, heads, &process, reference)
}

fn drawing_memory_verdict_with_process(
    phase: DrawingPhase,
    expected_heads: usize,
    heads: &[DrawingHeadObservation],
    process: &ProcessRenderReconciliation,
    reference: Option<&[DrawingHeadObservation]>,
) -> Result<(), String> {
    if process.retained_present_resource_count != 0 {
        return Err(format!(
            "CPU drawing target has {} retained GPU present resources ({} logical bytes)",
            process.retained_present_resource_count,
            process.retained_present_logical_gpu_bytes()
        ));
    }
    if heads.len() != expected_heads {
        return Err(format!(
            "expected {expected_heads} heads but observed {}",
            heads.len()
        ));
    }
    if matches!(
        phase,
        DrawingPhase::Freeze | DrawingPhase::Detached | DrawingPhase::Reattach
    ) && reference.is_none()
    {
        return Err(format!("{phase:?} requires a reference sample"));
    }

    for head in heads {
        let prior = reference.and_then(|rows| rows.iter().find(|row| row.name == head.name));
        if !head.status_present {
            return Err(format!("{}: missing status row", head.name));
        }
        if !head.connected || !head.ingame || head.scene_state != 2 {
            return Err(format!("{}: stopped or outside the live scene", head.name));
        }
        if head.login_started.is_none() {
            return Err(format!("{}: missing live login session", head.name));
        }
        if let Some(refusal) = &head.refusal {
            return Err(format!("{}: render head refused: {refusal}", head.name));
        }
        if head
            .owner_bytes
            .is_some_and(|bytes| bytes.gpu_retained_present() != 0)
        {
            return Err(format!(
                "{}: CPU head reports retained GPU present bytes",
                head.name
            ));
        }

        match phase {
            DrawingPhase::Headless | DrawingPhase::Detached => {
                if head.head_identity.is_some() {
                    return Err(format!("{}: head remained attached", head.name));
                }
                if head.retained_storage_bytes != 0 || head.owner_cpu_retained_bytes != Some(0) {
                    return Err(format!(
                        "{}: retained owner bytes remain after detach",
                        head.name
                    ));
                }
                if head
                    .owner_bytes
                    .is_none_or(|bytes| bytes.head_total() != 0 || bytes.process_total() != 0)
                {
                    return Err(format!(
                        "{}: no-head render-owner census was not all zero",
                        head.name
                    ));
                }
                if phase == DrawingPhase::Headless && head.frame_generation != 0 {
                    return Err(format!("{}: headless slot published a frame", head.name));
                }
                if phase == DrawingPhase::Detached {
                    let prior = prior.expect("validated phase reference");
                    if head.stream_epoch != prior.stream_epoch
                        || head.login_started != prior.login_started
                    {
                        return Err(format!(
                            "{}: detach changed the stream epoch or login session",
                            head.name
                        ));
                    }
                }
            }
            DrawingPhase::Live | DrawingPhase::Freeze | DrawingPhase::Reattach => {
                let identity = head
                    .head_identity
                    .ok_or_else(|| format!("{}: missing live render head", head.name))?;
                if head.stats_identity != Some(identity) {
                    return Err(format!(
                        "{}: fair stats do not match the live head generation",
                        head.name
                    ));
                }
                if head.backend != Some(BackendKind::Cpu) {
                    return Err(format!("{}: live backend is not CPU", head.name));
                }
                if head.paint_delta == 0 {
                    return Err(format!("{}: head stopped painting", head.name));
                }
                let Some(output) = head.output else {
                    return Err(format!("{}: no consumed frame output", head.name));
                };
                if output.kind != "pixmap"
                    || !output.fresh
                    || output.fault.is_some()
                    || output.width != Some(client::client::APPLET_W)
                    || output.height != Some(client::client::APPLET_H)
                    || output.pixels
                        != Some(
                            client::client::APPLET_W as usize * client::client::APPLET_H as usize,
                        )
                {
                    return Err(format!(
                        "{}: frame output is not a fresh complete CPU PixMap",
                        head.name
                    ));
                }
                if !head.has_valid_scene {
                    return Err(format!(
                        "{}: head painted without a complete retained scene",
                        head.name
                    ));
                }
                if phase != DrawingPhase::Freeze
                    && head.scene_progress == Some(SceneProgress::NoScene)
                {
                    return Err(format!(
                        "{}: head painted without complete scene progress",
                        head.name
                    ));
                }
                if phase == DrawingPhase::Freeze
                    && head.scene_progress != Some(SceneProgress::Retained)
                {
                    return Err(format!(
                        "{}: loading paint did not retain its complete scene",
                        head.name
                    ));
                }
                if head.retained_storage_allocations != 1 {
                    return Err(format!(
                        "{}: expected one retained allocation, observed {}",
                        head.name, head.retained_storage_allocations
                    ));
                }
                if head.retained_storage_bytes == 0
                    || head.owner_cpu_retained_bytes != Some(head.retained_storage_bytes)
                {
                    return Err(format!(
                        "{}: retained storage and owner census disagree",
                        head.name
                    ));
                }

                if phase == DrawingPhase::Freeze {
                    let prior = prior.expect("validated phase reference");
                    if head.head_identity != prior.head_identity {
                        return Err(format!("{}: freeze changed head generation", head.name));
                    }
                    if head.scene_consumes_delta != Some(0)
                        || head.scene_consumes != prior.scene_consumes
                    {
                        return Err(format!("{}: freeze consumed a new scene", head.name));
                    }
                    if head.loading_paints <= prior.loading_paints {
                        return Err(format!(
                            "{}: no loading paint reached the real head",
                            head.name
                        ));
                    }
                    if head.stream_epoch != prior.stream_epoch
                        || head.login_started != prior.login_started
                    {
                        return Err(format!(
                            "{}: freeze changed the stream epoch or login session",
                            head.name
                        ));
                    }
                    if output.generation <= prior.output.map_or(0, |value| value.generation) {
                        return Err(format!(
                            "{}: freeze produced no new frame output",
                            head.name
                        ));
                    }
                }
                if phase == DrawingPhase::Reattach {
                    let prior = prior.expect("validated phase reference");
                    let old = prior
                        .head_identity
                        .ok_or_else(|| format!("{}: reference had no old head", head.name))?;
                    if identity.slot != old.slot || identity.generation == old.generation {
                        return Err(format!(
                            "{}: reattach did not create a new generation in the same slot",
                            head.name
                        ));
                    }
                    if head.stream_epoch != prior.stream_epoch
                        || head.login_started != prior.login_started
                    {
                        return Err(format!(
                            "{}: reattach changed the stream epoch or login session",
                            head.name
                        ));
                    }
                    if output.generation <= prior.output.map_or(0, |value| value.generation) {
                        return Err(format!(
                            "{}: reattach produced no new frame output",
                            head.name
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

fn render_bytes_json(bytes: RenderBytes) -> Value {
    json!({
        "accounting_scope": "head_total excludes shared process rows; process_total is counted once per process; capture excludes shared host mesh; context_cpu includes shared Media",
        "cpu_surfaces": bytes.cpu_surfaces(),
        "cpu_retained_3d": bytes.cpu_retained_3d(),
        "output_pixels": bytes.output_pixels(),
        "capture_local_non_host_bytes": bytes.capture(),
        "nav_vertices": bytes.nav_vertices(),
        "nav_quads": bytes.nav_quads(),
        "nav_edges": bytes.nav_edges(),
        "nav_input": bytes.nav_input(),
        "head_models": bytes.head_models(),
        "head_misc": bytes.head_misc(),
        "temp_models": bytes.temp_models(),
        "shared_models": bytes.shared_models(),
        "shared_sprites": bytes.shared_sprites(),
        "gpu_fixed_textures": bytes.gpu_fixed_textures(),
        "gpu_fixed_buffers": bytes.gpu_fixed_buffers(),
        "gpu_scene_vertex_process_live": bytes.gpu_vertex_live(),
        "gpu_scene_vertex_process_retired": bytes.gpu_vertex_retired(),
        "gpu_nav_target": bytes.gpu_nav_target(),
        "gpu_nav_vertices": bytes.gpu_nav_vertices(),
        "gpu_retained_present_per_backend_gate_bytes": bytes.gpu_retained_present(),
        "gpu_upload_scratch": bytes.gpu_upload_scratch(),
        "context_cpu": bytes.context_cpu(),
        "context_gpu": bytes.context_gpu(),
        "scalar_state": bytes.scalar_state(),
        "backend_head_total_bytes": bytes.head_total(),
        "process_shared_total_bytes": bytes.process_total(),
    })
}

fn scalar_layout_receipt_json() -> Value {
    let bytes = render_head::test_hooks::scalar_receipt();
    json!({
        "scope": "release layout envelopes, not allocator-live observations",
        "renderer_backend_inline": "already included in each backend scalar_state row",
        "s_head": bytes.s_head(),
        "s_gate_per_gpu_head": bytes.s_gate(),
        "s_consumer_active_excluding_gate": bytes.s_consumer_active_bytes(),
        "s_consumer_draining_including_retired_gate": bytes.s_consumer(),
        "s_service_once": bytes.s_service(),
        "mailbox_per_loaded_bot_bytes": bytes.mailbox_per_loaded_bot_bytes(),
        "head_inline_excluding_renderer": bytes.head_inline_bytes(),
        "head_thread_local_bytes": bytes.head_thread_local_bytes(),
        "head_wake_arc_allocation_bytes": bytes.head_wake_arc_allocation_bytes(),
        "head_cell_arc_allocation_bytes": bytes.head_cell_arc_allocation_bytes(),
        "head_budget_state_allocation_count": bytes.head_budget_state_allocation_count(),
        "head_budget_arc_allocation_bytes": bytes.head_budget_arc_allocation_bytes(),
        "render_process_static_bytes": bytes.render_process_static_bytes(),
        "fair_process_static_bytes": bytes.fair_process_static_bytes(),
        "render_service_arc_allocation_bytes": bytes.render_service_arc_allocation_bytes(),
        "process_budget_state_allocation_count": bytes.process_budget_state_allocation_count(),
        "process_budget_arc_allocation_bytes": bytes.process_budget_arc_allocation_bytes(),
        "present_gate_arc_allocation_bytes": bytes.present_gate_arc_allocation_bytes(),
        "present_retained_arc_allocation_bytes": bytes.present_retained_arc_allocation_bytes(),
        "present_registration_descriptor_bytes": bytes.present_registration_descriptor_bytes(),
        "present_held_frame_descriptor_bytes": bytes.present_held_frame_descriptor_bytes(),
        "present_draining_gate_arc_allocation_bytes": bytes.present_draining_gate_arc_allocation_bytes(),
        "present_process_service_bytes": bytes.present_process_service_bytes(),
        "counting_rule": "S_head per attached head; gate per live GPU head; consumer active or draining, never both; service once; mailbox per loaded bot",
    })
}

fn v8_sum(rows: &[Value], key: &str) -> u64 {
    rows.iter()
        .filter_map(|row| row.get(key).and_then(Value::as_u64))
        .fold(0u64, |total, value| total.saturating_add(value))
}

fn wait_owner_sample(mailbox: &FrameBuf) -> RenderBytes {
    let deadline = Instant::now() + OWNER_SAMPLE_TIMEOUT;
    loop {
        if let Some(sample) = mailbox.render_bytes_sample() {
            return sample;
        }
        assert!(
            Instant::now() < deadline,
            "render-owner census did not complete"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn wait_frame_outputs(heads: &[HeadProbe], previous: Option<&[DrawingHeadObservation]>) {
    let deadline = Instant::now() + OWNER_SAMPLE_TIMEOUT;
    loop {
        let ready = heads.iter().all(|head| {
            if head.mailbox.head().is_none() {
                return true;
            }
            let Some(output) = head.output.latest() else {
                return false;
            };
            let previous_generation = previous
                .and_then(|rows| rows.iter().find(|row| row.name == head.name))
                .and_then(|row| row.output)
                .map_or(0, |output| output.generation);
            output.generation > previous_generation
        });
        if ready {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "a live CPU head did not publish a newly consumed PixMap"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn observation_for(
    head: &HeadProbe,
    owner_bytes: RenderBytes,
    statuses: &[host_play::SlotStatus],
    previous: Option<&[DrawingHeadObservation]>,
) -> DrawingHeadObservation {
    let status = statuses.iter().find(|status| status.username == head.name);
    let current_identity = head.mailbox.head().map(head_key);
    let raw_stats = head.mailbox.fair_stats();
    let last_stats_identity = raw_stats.map(|stats| head_key(stats.identity()));
    let active_stats =
        raw_stats.filter(|stats| Some(head_key(stats.identity())) == current_identity);
    let stats_identity = active_stats.map(|stats| head_key(stats.identity()));
    let prior = previous.and_then(|rows| rows.iter().find(|row| row.name == head.name));
    let frame_generation = head.mailbox.generation();
    let paint_delta = prior.map_or(frame_generation, |prior| {
        frame_generation.saturating_sub(prior.frame_generation)
    });
    let scene_consumes = active_stats.map_or(0, |stats| stats.scene_consumes());
    let scene_consumes_delta = match (active_stats, prior) {
        (Some(_), Some(prior)) if prior.stats_identity == stats_identity => {
            Some(scene_consumes.saturating_sub(prior.scene_consumes))
        }
        (Some(stats), None) => Some(stats.scene_consumes()),
        _ => None,
    };
    DrawingHeadObservation {
        name: head.name.clone(),
        status_present: status.is_some(),
        connected: status.is_some_and(|status| status.connected),
        ingame: status.is_some_and(|status| status.ingame),
        scene_state: status.map_or(0, |status| status.scene_state),
        stream_epoch: status.map_or(0, |status| status.stream_epoch),
        login_started: status.and_then(|status| status.login_started),
        head_identity: current_identity,
        last_stats_identity,
        stats_identity,
        backend: active_stats.map(|stats| stats.backend()),
        frame_generation,
        paint_delta,
        output: head.output.latest(),
        scene_consumes,
        scene_consumes_delta,
        has_valid_scene: active_stats.is_some_and(|stats| stats.has_valid_scene()),
        scene_progress: active_stats.map(|stats| stats.scene_progress()),
        retained_storage_allocations: active_stats
            .map_or(0, |stats| stats.retained_storage_allocations()),
        retained_storage_bytes: active_stats.map_or(0, |stats| stats.retained_storage_bytes()),
        owner_bytes: Some(owner_bytes),
        owner_cpu_retained_bytes: Some(owner_bytes.cpu_retained_3d()),
        refusal: head.mailbox.refusal().map(|refusal| format!("{refusal:?}")),
        loading_paints: head.control.loading_paints.load(Ordering::Relaxed),
    }
}

fn capture_sample(
    phase: &str,
    sample_index: usize,
    phase_started: Instant,
    play: &Play,
    heads: &[HeadProbe],
    previous: Option<&[DrawingHeadObservation]>,
) -> (
    Vec<DrawingHeadObservation>,
    ProcessRenderReconciliation,
    Value,
) {
    for head in heads {
        head.mailbox.request_render_bytes_sample();
    }
    let owner_bytes: Vec<_> = heads
        .iter()
        .map(|head| wait_owner_sample(&head.mailbox))
        .collect();
    wait_frame_outputs(heads, previous);
    let statuses = play.statuses();
    let observations: Vec<_> = heads
        .iter()
        .zip(owner_bytes)
        .map(|(head, bytes)| observation_for(head, bytes, &statuses, previous))
        .collect();
    let process_render = reconcile_process_render(
        ProcessRenderCensus {
            shared_host_capture_storage_bytes:
                client::render::backend::gpu::process_capture_storage_bytes(),
            media_process_storage_bytes: client::render::media::Media::process_storage_bytes(),
            retained_present_resource_count:
                client::render::backend::present::process_retained_present_resource_count(),
        },
        &observations,
    );
    let (allocation_calls, allocated_bytes, live_bytes) = rust_allocator_counts();
    let v8_rows = script_js::memory_profile::snapshots();
    let sample = json!({
        "phase": phase,
        "phase_sample": sample_index,
        "phase_elapsed_s": phase_started.elapsed().as_secs_f64(),
        "process": {
            "rss_bytes": current_resident_bytes(),
            "native_allocator": native_allocator_sample(),
            "rust_allocator": {
                "allocation_calls": allocation_calls,
                "allocated_bytes": allocated_bytes,
                "live_bytes": live_bytes,
            },
            "render_owner_reconciliation": process_render.json(),
            "render_scalar_layout": scalar_layout_receipt_json(),
            "v8_domains": {
                "snapshot_inflight_bytes": v8_sum(&v8_rows, "snapshot_inflight_bytes"),
                "snapshot_inflight_capacity": v8_sum(&v8_rows, "snapshot_inflight_capacity"),
                "snapshot_peak_capacity_sum": v8_sum(&v8_rows, "snapshot_peak_capacity"),
                "v8_live": v8_sum(&v8_rows, "v8_live"),
                "v8_used_bytes": v8_sum(&v8_rows, "v8_used_bytes"),
                "v8_total_bytes": v8_sum(&v8_rows, "v8_total_bytes"),
                "v8_heap_samples": v8_sum(&v8_rows, "v8_heap_samples"),
                "v8_updated_ms": v8_sum(&v8_rows, "v8_updated_ms"),
                "script_tick_count": v8_sum(&v8_rows, "script_tick_count"),
                "script_tick_total_ns": v8_sum(&v8_rows, "script_tick_total_ns"),
                "script_tick_max_ns_sum": v8_sum(&v8_rows, "script_tick_max_ns"),
                "isolates": v8_rows,
            },
        },
        "heads": observations.iter().map(DrawingHeadObservation::json).collect::<Vec<_>>(),
    });
    (observations, process_render, sample)
}

fn validate_sample(
    phase: DrawingPhase,
    expected_heads: usize,
    observations: &[DrawingHeadObservation],
    process_render: &ProcessRenderReconciliation,
    reference: Option<&[DrawingHeadObservation]>,
) {
    if let Err(error) = drawing_memory_verdict_with_process(
        phase,
        expected_heads,
        observations,
        process_render,
        reference,
    ) {
        panic!("{phase:?} drawing-memory verdict failed: {error}");
    }
}

fn wait_ingame(play: &Play, heads: usize, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        let statuses = play.statuses();
        let ingame = statuses
            .iter()
            .filter(|status| status.connected && status.ingame && status.scene_state == 2)
            .count();
        if statuses.len() >= heads && ingame >= heads {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "only {ingame}/{heads} drawing-memory slots reached a live scene"
        );
        thread::sleep(Duration::from_millis(100));
    }
}

fn wait_live_heads(play: &Play, heads: &[HeadProbe], old: Option<&[DrawingHeadObservation]>) {
    let deadline = Instant::now() + Duration::from_secs(60 + 30 * heads.len() as u64);
    loop {
        let statuses = play.statuses();
        let ready = heads.iter().all(|head| {
            let status = statuses
                .iter()
                .find(|status| status.username == head.name)
                .is_some_and(|status| status.connected && status.ingame && status.scene_state == 2);
            let identity = head.mailbox.head().map(head_key);
            let stats = head.mailbox.fair_stats();
            let complete = identity.is_some()
                && stats.is_some_and(|stats| {
                    Some(head_key(stats.identity())) == identity
                        && stats.backend() == BackendKind::Cpu
                        && stats.has_valid_scene()
                        && stats.retained_storage_allocations() == 1
                        && stats.retained_storage_bytes() > 0
                });
            let new_generation = old.is_none_or(|rows| {
                rows.iter()
                    .find(|row| row.name == head.name)
                    .and_then(|row| row.head_identity.zip(identity))
                    .is_some_and(|(before, after)| {
                        before.slot == after.slot && before.generation != after.generation
                    })
            });
            status && complete && new_generation
        });
        if ready {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "not every CPU drawing head attached with a complete scene"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

fn set_policy(heads: &[HeadProbe], policy: &str, focus_index: usize) {
    for (index, head) in heads.iter().enumerate() {
        let focused = index == focus_index;
        head.input.set_render_focused(focused);
        head.input.set_full_rate(policy == "full" || focused);
    }
}
fn wake_slots(play: &Play, heads: &[HeadProbe]) {
    for head in heads {
        play.wake(&head.name);
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Each measurement phase supplies its policy, timing, live heads and prior owner census"
)]
fn run_live_policy(
    label: &str,
    policy: &str,
    focus_index: usize,
    warmup: Duration,
    observe: Duration,
    expected_heads: usize,
    play: &Play,
    heads: &[HeadProbe],
    initial_previous: Option<&[DrawingHeadObservation]>,
) -> (Vec<Value>, Vec<DrawingHeadObservation>) {
    set_policy(heads, policy, focus_index);
    wake_slots(play, heads);
    thread::sleep(warmup);
    let phase_started = Instant::now();
    let mut samples = Vec::new();
    let (mut last, process_render, sample) =
        capture_sample(label, 0, phase_started, play, heads, initial_previous);
    validate_sample(
        DrawingPhase::Live,
        expected_heads,
        &last,
        &process_render,
        None,
    );
    samples.push(sample);
    let mut sample_index = 1;
    while phase_started.elapsed() < observe {
        thread::sleep(SAMPLE_PERIOD);
        let (current, process_render, sample) =
            capture_sample(label, sample_index, phase_started, play, heads, Some(&last));
        validate_sample(
            DrawingPhase::Live,
            expected_heads,
            &current,
            &process_render,
            None,
        );
        last = current;
        samples.push(sample);
        sample_index += 1;
    }
    (samples, last)
}

fn run_loading_freeze(
    expected_heads: usize,
    observe: Duration,
    play: &Play,
    heads: &[HeadProbe],
    baseline: &[DrawingHeadObservation],
) -> (Vec<Value>, Vec<DrawingHeadObservation>) {
    let loading = Arc::clone(&heads[0].control.loading);
    loading.store(true, Ordering::SeqCst);
    wake_slots(play, heads);
    let phase_started = Instant::now();
    let mut samples = Vec::new();
    let (mut last, process_render, sample) = capture_sample(
        "loading_freeze",
        0,
        phase_started,
        play,
        heads,
        Some(baseline),
    );
    validate_sample(
        DrawingPhase::Freeze,
        expected_heads,
        &last,
        &process_render,
        Some(baseline),
    );
    samples.push(sample);
    let mut sample_index = 1;
    while phase_started.elapsed() < observe {
        thread::sleep(SAMPLE_PERIOD);
        let (current, process_render, sample) = capture_sample(
            "loading_freeze",
            sample_index,
            phase_started,
            play,
            heads,
            Some(&last),
        );
        validate_sample(
            DrawingPhase::Freeze,
            expected_heads,
            &current,
            &process_render,
            Some(baseline),
        );
        last = current;
        samples.push(sample);
        sample_index += 1;
    }
    loading.store(false, Ordering::SeqCst);
    wake_slots(play, heads);
    (samples, last)
}

fn live_profiles(names: &[String]) -> Vec<Profile> {
    names
        .iter()
        .enumerate()
        .map(|(index, name)| Profile {
            username: name.clone(),
            password: name.clone().into(),
            uid: 274_300_000 + index as i32,
            settings: ProfileSettings {
                auto_login: true,
                lowmem: true,
                raster: RasterMode::Cpu,
                ..ProfileSettings::default()
            },
        })
        .collect()
}

fn memory_verdict_fixture(name: &str, generation: u64) -> DrawingHeadObservation {
    let identity = HeadKey {
        slot: 7,
        generation,
    };
    DrawingHeadObservation {
        name: name.into(),
        status_present: true,
        connected: true,
        ingame: true,
        scene_state: 2,
        stream_epoch: 4,
        login_started: Some(Instant::now()),
        head_identity: Some(identity),
        last_stats_identity: Some(identity),
        stats_identity: Some(identity),
        backend: Some(BackendKind::Cpu),
        frame_generation: 9,
        paint_delta: 1,
        output: Some(OutputObservation {
            kind: "pixmap",
            width: Some(client::client::APPLET_W),
            height: Some(client::client::APPLET_H),
            pixels: Some(client::client::APPLET_W as usize * client::client::APPLET_H as usize),
            fresh: true,
            fault: None,
            generation: 9,
        }),
        scene_consumes: 0,
        scene_consumes_delta: Some(0),
        has_valid_scene: true,
        scene_progress: Some(SceneProgress::Rebuilt),
        retained_storage_allocations: 1,
        retained_storage_bytes: 684_032,
        owner_bytes: None,
        owner_cpu_retained_bytes: Some(684_032),
        refusal: None,
        loading_paints: 0,
    }
}

#[test]
fn drawing_memory_oracle_rejects_missing_heads_and_copy_leaks() {
    let expected = 1;
    let no_gpu_process = reconcile_process_render(
        ProcessRenderCensus {
            shared_host_capture_storage_bytes: 4096,
            media_process_storage_bytes: 4096,
            retained_present_resource_count: 0,
        },
        &[],
    );
    assert_eq!(no_gpu_process.capture_storage_bytes(), 4096);
    assert_eq!(no_gpu_process.retained_present_logical_gpu_bytes(), 0);
    assert_eq!(no_gpu_process.max_head_context_cpu_bytes, 0);
    assert_eq!(no_gpu_process.context_cpu_bytes(), 4096);
    assert_eq!(
        no_gpu_process.json()["media_process_storage_bytes"],
        json!(4096)
    );
    assert_eq!(
        no_gpu_process.json()["process_context_cpu_bytes"],
        json!(4096)
    );
    let live_without_backend_gate = memory_verdict_fixture("cpu-1", 1);
    let draining_gpu_process = reconcile_process_render(
        ProcessRenderCensus {
            shared_host_capture_storage_bytes: 4096,
            media_process_storage_bytes: 4096,
            retained_present_resource_count: 1,
        },
        std::slice::from_ref(&live_without_backend_gate),
    );
    assert_eq!(
        draining_gpu_process.retained_present_logical_gpu_bytes(),
        RETAINED_PRESENT_GPU_BYTES_PER_RESOURCE
    );
    assert!(drawing_memory_verdict_with_process(
        DrawingPhase::Headless,
        expected,
        &[live_without_backend_gate],
        &draining_gpu_process,
        None
    )
    .unwrap_err()
    .contains("retained GPU present resources"));
    let mut live = memory_verdict_fixture("cpu-1", 1);
    assert!(drawing_memory_verdict(DrawingPhase::Live, expected, &[live.clone()], None).is_ok());
    assert!(
        drawing_memory_verdict(DrawingPhase::Live, expected, &[], None)
            .unwrap_err()
            .contains("expected 1 heads")
    );

    let mut stopped = live.clone();
    stopped.ingame = false;
    assert!(
        drawing_memory_verdict(DrawingPhase::Live, expected, &[stopped], None)
            .unwrap_err()
            .contains("stopped")
    );
    let mut missing_attachment = live.clone();
    missing_attachment.head_identity = None;
    assert!(
        drawing_memory_verdict(DrawingPhase::Live, expected, &[missing_attachment], None)
            .unwrap_err()
            .contains("missing live render head")
    );

    live.refusal = Some("HeadCap".into());
    assert!(
        drawing_memory_verdict(DrawingPhase::Live, expected, &[live.clone()], None)
            .unwrap_err()
            .contains("refused")
    );
    live.refusal = None;

    live.has_valid_scene = false;
    assert!(
        drawing_memory_verdict(DrawingPhase::Live, expected, &[live.clone()], None)
            .unwrap_err()
            .contains("without a complete retained scene")
    );
    live.has_valid_scene = true;

    live.retained_storage_allocations = 2;
    assert!(
        drawing_memory_verdict(DrawingPhase::Live, expected, &[live.clone()], None)
            .unwrap_err()
            .contains("expected one retained allocation")
    );
    live.retained_storage_allocations = 1;

    live.output = Some(OutputObservation {
        kind: "pixmap",
        width: Some(client::client::APPLET_W),
        height: Some(client::client::APPLET_H),
        pixels: Some(1),
        fresh: true,
        fault: None,
        generation: 9,
    });
    assert!(
        drawing_memory_verdict(DrawingPhase::Live, expected, &[live.clone()], None)
            .unwrap_err()
            .contains("complete CPU PixMap")
    );

    let mut freeze_baseline = memory_verdict_fixture("cpu-1", 1);
    freeze_baseline.loading_paints = 0;
    let mut frozen = freeze_baseline.clone();
    frozen.loading_paints = 1;
    frozen.scene_progress = Some(SceneProgress::Retained);
    frozen.output = Some(OutputObservation {
        generation: 10,
        ..frozen.output.unwrap()
    });
    assert!(drawing_memory_verdict(
        DrawingPhase::Freeze,
        expected,
        &[frozen.clone()],
        Some(&[freeze_baseline.clone()])
    )
    .is_ok());
    frozen.scene_consumes = 1;
    frozen.scene_consumes_delta = Some(1);
    assert!(drawing_memory_verdict(
        DrawingPhase::Freeze,
        expected,
        &[frozen],
        Some(&[freeze_baseline])
    )
    .unwrap_err()
    .contains("consumed a new scene"));

    let old = memory_verdict_fixture("cpu-1", 1);
    let mut detached = old.clone();
    detached.head_identity = None;
    detached.stats_identity = None;
    detached.backend = None;
    detached.output = None;
    detached.scene_consumes = 0;
    detached.scene_progress = None;
    detached.retained_storage_allocations = 0;
    detached.retained_storage_bytes = 0;
    detached.owner_cpu_retained_bytes = Some(684_032);
    detached.refusal = None;
    assert!(drawing_memory_verdict(
        DrawingPhase::Detached,
        expected,
        &[detached.clone()],
        Some(std::slice::from_ref(&old))
    )
    .unwrap_err()
    .contains("retained owner bytes"));

    let mut detached_zero = detached;
    detached_zero.owner_bytes = Some(RenderBytes::default());
    detached_zero.owner_cpu_retained_bytes = Some(0);
    assert!(drawing_memory_verdict(
        DrawingPhase::Detached,
        expected,
        &[detached_zero.clone()],
        Some(std::slice::from_ref(&old))
    )
    .is_ok());

    let mut headless_zero = detached_zero.clone();
    headless_zero.frame_generation = 0;
    assert!(drawing_memory_verdict(
        DrawingPhase::Headless,
        expected,
        &[headless_zero.clone()],
        None
    )
    .is_ok());
    headless_zero.owner_bytes = None;
    assert!(
        drawing_memory_verdict(DrawingPhase::Headless, expected, &[headless_zero], None)
            .unwrap_err()
            .contains("no-head render-owner census was not all zero")
    );

    let mut reattached = old.clone();
    reattached.head_identity = Some(HeadKey {
        slot: 7,
        generation: 2,
    });
    reattached.last_stats_identity = reattached.head_identity;
    reattached.stats_identity = reattached.head_identity;
    reattached.output = Some(OutputObservation {
        generation: 10,
        ..reattached.output.unwrap()
    });
    assert!(drawing_memory_verdict(
        DrawingPhase::Reattach,
        expected,
        &[reattached.clone()],
        Some(std::slice::from_ref(&old))
    )
    .is_ok());
    reattached.head_identity = old.head_identity;
    reattached.stats_identity = old.stats_identity;
    assert!(drawing_memory_verdict(
        DrawingPhase::Reattach,
        expected,
        &[reattached],
        Some(&[old])
    )
    .unwrap_err()
    .contains("new generation"));
}

#[test]
#[ignore = "LIVE=1 Engine A CPU drawing-memory measurement"]
fn t44_cpu_draw_memory_ab() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let n: usize = required("M3_N").parse().expect("M3_N must be an integer");
    assert!(n > 0, "M3_N must be positive");
    let backend = std::env::var("M3_BACKEND").unwrap_or_else(|_| "cpu".into());
    assert_eq!(
        backend, "cpu",
        "t44_cpu_draw_memory_ab only accepts CPU heads"
    );
    assert_eq!(
        std::env::var("BOT_CPU").as_deref(),
        Ok("1"),
        "BOT_CPU=1 is required for the CPU drawing-memory target"
    );
    let focus_index = env_or("M3_FOCUS_INDEX", 0usize);
    assert!(focus_index < n, "M3_FOCUS_INDEX must name a live head");
    let warmup = Duration::from_secs(env_or("M3_WARMUP_S", 10u64));
    let observe = Duration::from_secs(env_or("M3_OBSERVE_S", 30u64));
    assert!(!warmup.is_zero(), "M3_WARMUP_S must be positive");
    assert!(!observe.is_zero(), "M3_OBSERVE_S must be positive");
    let policies: Vec<String> = std::env::var("M3_POLICIES")
        .unwrap_or_else(|_| "full,focus1".into())
        .split(',')
        .map(|policy| policy.trim().to_owned())
        .collect();
    assert_eq!(
        policies.iter().filter(|policy| *policy == "full").count(),
        1,
        "M3_POLICIES must include full exactly once"
    );
    assert_eq!(
        policies.iter().filter(|policy| *policy == "focus1").count(),
        1,
        "M3_POLICIES must include focus1 exactly once"
    );
    assert_eq!(policies.len(), 2, "only full and focus1 are supported");

    let output = PathBuf::from(required("M3_OUT"));
    std::fs::create_dir_all(&output).expect("create M3_OUT");
    client::profiling::enable();
    std::sync::LazyLock::force(&EPOCH);
    configure_drawing_fixture();

    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(env_or("M3_PORT", 44594u16)),
        http_port: Some(env_or("M3_HTTP_PORT", 1080u16)),
        engine_dir: Some(required("ENGINE_DIR").into()),
        nav_pack: Some(required("NAV_PACK").into()),
        ..ProfileOptions::default()
    };
    let bound = options
        .resolve(None)
        .expect("resolve local Engine A profile")
        .bind_runtime()
        .expect("bind local Engine A profile");
    let names = mint_live_names(n);
    let draw = Arc::new(AtomicBool::new(false));
    let loading = Arc::new(AtomicBool::new(false));
    let probes: Vec<_> = names
        .iter()
        .map(|name| {
            let input = SlotInput::new();
            input.set_prefer_cpu(true);
            HeadProbe {
                name: name.clone(),
                input,
                mailbox: FrameBuf::new(),
                control: PaintControl {
                    loading: Arc::clone(&loading),
                    loading_paints: Arc::new(AtomicU64::new(0)),
                },
                output: Arc::new(OutputProbe::default()),
            }
        })
        .collect();
    let template = SharedClientTemplate::load(bound).expect("load shared client template");
    let frame_draw = Arc::clone(&draw);
    let mut play = run_with_template(
        template,
        false,
        Vec::new(),
        |_| (None, None),
        move |client, _, _| client.set_draw(frame_draw.load(Ordering::Relaxed)),
    )
    .expect("create real Play from the M3 template");

    let profiles = live_profiles(&names);
    for (profile, head) in profiles.into_iter().zip(&probes) {
        play.set_render_test_control(&head.name, Arc::new(head.control.clone()))
            .expect("bind named paint-only control before slot spawn");
        play.try_spawn_slot(
            profile,
            Some(Arc::clone(&head.input)),
            Some(Arc::clone(&head.mailbox)),
            None,
        )
        .expect("spawn real CPU Play slot");
    }

    let stop = Arc::new(AtomicBool::new(false));
    let consumer = {
        let thread_stop = Arc::clone(&stop);
        let mailboxes: Vec<_> = probes
            .iter()
            .map(|head| (Arc::clone(&head.mailbox), Arc::clone(&head.output)))
            .collect();
        let join = thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                for (mailbox, output) in &mailboxes {
                    if let Some(frame) = mailbox.take() {
                        output.observe(mailbox, &frame);
                    }
                }
                render_head::pump(Instant::now());
                thread::sleep(CONSUMER_PERIOD);
            }
        });
        ConsumerGuard {
            stop: Arc::clone(&stop),
            join: Some(join),
        }
    };

    wait_ingame(&play, n, Duration::from_secs(60 + 30 * n as u64));
    thread::sleep(warmup);
    let headless_started = Instant::now();
    let (headless_heads, headless_process_render, headless_sample) =
        capture_sample("headless", 0, headless_started, &play, &probes, None);
    validate_sample(
        DrawingPhase::Headless,
        n,
        &headless_heads,
        &headless_process_render,
        None,
    );

    draw.store(true, Ordering::Relaxed);
    set_policy(&probes, "full", focus_index);
    wake_slots(&play, &probes);
    wait_live_heads(&play, &probes, None);
    let draw_on_started = Instant::now();
    let (draw_on_heads, draw_on_process_render, draw_on_sample) = capture_sample(
        "draw_on",
        0,
        draw_on_started,
        &play,
        &probes,
        Some(&headless_heads),
    );
    validate_sample(
        DrawingPhase::Live,
        n,
        &draw_on_heads,
        &draw_on_process_render,
        None,
    );

    let mut phases = serde_json::Map::new();
    phases.insert("headless".into(), json!({"samples": [headless_sample]}));
    phases.insert("draw_on".into(), json!({"samples": [draw_on_sample]}));
    let mut last_live = draw_on_heads;
    for policy in policies {
        let label = format!("warm_{policy}");
        let (samples, last) = run_live_policy(
            &label,
            &policy,
            focus_index,
            warmup,
            observe,
            n,
            &play,
            &probes,
            Some(&last_live),
        );
        phases.insert(
            label,
            json!({
                "warmup_s": warmup.as_secs(),
                "observe_s": observe.as_secs(),
                "samples": samples,
            }),
        );
        last_live = last;
    }

    let (freeze_samples, last_frozen) = run_loading_freeze(n, observe, &play, &probes, &last_live);
    phases.insert(
        "loading_freeze".into(),
        json!({
            "duration_s": observe.as_secs(),
            "samples": freeze_samples,
        }),
    );

    loading.store(false, Ordering::SeqCst);
    draw.store(false, Ordering::Relaxed);
    wake_slots(&play, &probes);
    let detach_deadline = Instant::now() + Duration::from_secs(60 + 30 * n as u64);
    while probes.iter().any(|head| head.mailbox.head().is_some()) {
        assert!(
            Instant::now() < detach_deadline,
            "draw-off did not detach every CPU head"
        );
        thread::sleep(Duration::from_millis(25));
    }
    let drained: Vec<bool> = probes
        .iter()
        .map(|head| head.mailbox.take().is_none())
        .collect();
    assert!(
        drained.iter().all(|drained| *drained),
        "draw-off must drain every retained frame: {drained:?}"
    );
    let detached_started = Instant::now();
    let (detached_first_heads, detached_first_process_render, detached_first) = capture_sample(
        "detached",
        0,
        detached_started,
        &play,
        &probes,
        Some(&last_frozen),
    );
    validate_sample(
        DrawingPhase::Detached,
        n,
        &detached_first_heads,
        &detached_first_process_render,
        Some(&last_frozen),
    );
    thread::sleep(Duration::from_secs(2));
    for (head, before) in probes.iter().zip(&detached_first_heads) {
        assert_eq!(
            head.mailbox.generation(),
            before.frame_generation,
            "{}: a detached mailbox published another paint",
            head.name
        );
    }
    let (detached_second_heads, detached_second_process_render, detached_second) = capture_sample(
        "detached",
        1,
        detached_started,
        &play,
        &probes,
        Some(&detached_first_heads),
    );
    validate_sample(
        DrawingPhase::Detached,
        n,
        &detached_second_heads,
        &detached_second_process_render,
        Some(&last_frozen),
    );
    assert!(
        detached_second_heads
            .iter()
            .all(|head| head.paint_delta == 0),
        "detached heads published a paint after drain"
    );
    phases.insert(
        "detached".into(),
        json!({
            "mailbox_drained": drained,
            "painted_while_detached": detached_second_heads
                .iter()
                .map(|head| head.paint_delta)
                .collect::<Vec<_>>(),
            "samples": [detached_first, detached_second],
        }),
    );

    draw.store(true, Ordering::Relaxed);
    set_policy(&probes, "full", focus_index);
    wake_slots(&play, &probes);
    wait_live_heads(&play, &probes, Some(&last_live));
    let reattach_started = Instant::now();
    let (reattached_heads, reattached_process_render, reattach_sample) = capture_sample(
        "reattached",
        0,
        reattach_started,
        &play,
        &probes,
        Some(&detached_second_heads),
    );
    validate_sample(
        DrawingPhase::Reattach,
        n,
        &reattached_heads,
        &reattached_process_render,
        Some(&last_live),
    );
    phases.insert("reattached".into(), json!({"samples": [reattach_sample]}));

    for head in &probes {
        if let Some(arm) = play.arm(&head.name) {
            arm.request_logout();
        }
    }
    let logout_deadline = Instant::now() + Duration::from_secs(60);
    while play.statuses().iter().any(|status| status.ingame) && Instant::now() < logout_deadline {
        thread::sleep(Duration::from_millis(250));
    }
    consumer.stop_and_join();

    let receipt = json!({
        "schema": "t44-cpu-draw-memory/v1",
        "qualification": "slice-4-phase-harness; no S5 A/A or A/B claim",
        "backend": "cpu",
        "n": n,
        "focus_index": focus_index,
        "warmup_s": warmup.as_secs(),
        "observe_s": observe.as_secs(),
        "git_rev": std::env::var("M3_GIT_REV").unwrap_or_default(),
        "phases": phases,
    });
    let path = output.join(format!("cpu-n{n}-draw-memory.json"));
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&receipt).expect("serialize drawing-memory evidence"),
    )
    .expect("write drawing-memory evidence");
    eprintln!("t44 CPU drawing-memory evidence: {}", path.display());
    assert!(
        !play.statuses().iter().any(|status| status.ingame),
        "every drawing-memory slot logged out"
    );
}

/// Rendering qualification: run separately for A=1 and A=q with an explicit cap.
#[test]
#[ignore = "requires live Engine A, GPU, explicit retained cap, and measurement directory"]
fn t44_gpu_retained_live() {
    use client::render::backend::{
        capture::CaptureGeometry,
        present::{self, ConsumerFrame, ReadbackKind},
        readback::{ReadbackBudget, ReadbackError},
        TextureHandle,
    };
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_ne!(std::env::var("BOT_CPU").as_deref(), Ok("1"));
    let n: usize = required("M3_N").parse().unwrap();
    let cap: usize = required("T44_RETAINED_CAP").parse().unwrap();
    assert!(n > 0 && (n == 1 || n == cap));
    present::configure_retained_present_budget(std::num::NonZeroUsize::new(cap).unwrap())
        .expect("configure explicit retained cap");
    let output = PathBuf::from(required("M3_OUT"));
    std::fs::create_dir_all(&output).unwrap();
    configure_drawing_fixture();
    client::profiling::enable();
    let bound = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(env_or("M3_PORT", 44594u16)),
        http_port: Some(env_or("M3_HTTP_PORT", 1080u16)),
        engine_dir: Some(required("ENGINE_DIR").into()),
        nav_pack: Some(required("NAV_PACK").into()),
        ..ProfileOptions::default()
    }
    .resolve(None)
    .unwrap()
    .bind_runtime()
    .unwrap();
    let names = mint_live_names(n);
    let probes: Vec<_> = names
        .iter()
        .map(|name| {
            let input = SlotInput::new();
            input.set_prefer_cpu(false);
            HeadProbe {
                name: name.clone(),
                input,
                mailbox: FrameBuf::new(),
                control: PaintControl {
                    loading: Arc::new(AtomicBool::new(false)),
                    loading_paints: Arc::new(AtomicU64::new(0)),
                },
                output: Arc::new(OutputProbe::default()),
            }
        })
        .collect();
    let textures: Vec<Arc<Mutex<Option<TextureHandle>>>> =
        (0..n).map(|_| Arc::new(Mutex::new(None))).collect();
    let draw = Arc::new(AtomicBool::new(true));
    let frame_draw = Arc::clone(&draw);
    let mut play = run_with_template(
        SharedClientTemplate::load(bound).unwrap(),
        false,
        Vec::new(),
        |_| (None, None),
        move |client, _, _| client.set_draw(frame_draw.load(Ordering::Relaxed)),
    )
    .unwrap();
    set_policy(&probes, "full", 0);
    for (profile, head) in live_profiles(&names).into_iter().zip(&probes) {
        play.try_spawn_slot(
            profile,
            Some(Arc::clone(&head.input)),
            Some(Arc::clone(&head.mailbox)),
            None,
        )
        .unwrap();
    }
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let mailboxes: Vec<_> = probes
        .iter()
        .zip(&textures)
        .map(|(h, texture)| {
            (
                Arc::clone(&h.mailbox),
                Arc::clone(&h.output),
                Arc::clone(texture),
            )
        })
        .collect();
    let consumer = ConsumerGuard {
        stop,
        join: Some(thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                for (mailbox, output, texture) in &mailboxes {
                    if let Some(frame) = mailbox.take() {
                        output.observe(mailbox, &frame);
                        if let FrameOutput::Texture(frame) = frame {
                            *texture.lock().unwrap() = Some(frame);
                        }
                    }
                }
                render_head::pump(Instant::now());
                thread::sleep(CONSUMER_PERIOD);
            }
        })),
    };
    wait_ingame(&play, n, Duration::from_secs(60 + 30 * n as u64));
    let wait_gpu = || {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !probes.iter().all(|head| {
            head.mailbox.fair_stats().is_some_and(|s| {
                s.backend() == BackendKind::Gpu
                    && s.has_valid_scene()
                    && head.mailbox.head().map(head_key) == Some(head_key(s.identity()))
            })
        }) {
            assert!(Instant::now() < deadline, "GPU scene deadline");
            thread::sleep(Duration::from_millis(50));
        }
    };
    wait_gpu();
    thread::sleep(Duration::from_secs(10));
    let register = |texture: &Mutex<Option<TextureHandle>>| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(frame) = texture.lock().unwrap().as_ref() {
                match frame.try_register_consumer() {
                    Ok(registration) => return registration,
                    Err(ReadbackError::Busy) => {}
                    Err(error) => panic!("register GPU retained consumer: {error:?}"),
                }
            }
            assert!(Instant::now() < deadline, "retained registration deadline");
            thread::sleep(Duration::from_millis(10));
        }
    };
    let mut receipts = Vec::new();
    let mut samples = Vec::new();
    let mut registrations: Vec<present::ConsumerRegistration> = Vec::new();
    let mut held: Vec<ConsumerFrame> = Vec::new();
    let mut readbacks = Vec::new();
    let mut overflow_refused = n != cap;
    for phase in [
        GpuRetainedPhase::Off,
        GpuRetainedPhase::Registered,
        GpuRetainedPhase::Held,
        GpuRetainedPhase::Draining,
        GpuRetainedPhase::Drained,
    ] {
        if phase == GpuRetainedPhase::Held {
            for registration in &registrations {
                let deadline = Instant::now() + Duration::from_secs(10);
                loop {
                    match registration.try_take_frame() {
                        Ok(Some(frame)) => {
                            assert!(frame.has_valid_scene());
                            held.push(frame);
                            break;
                        }
                        Ok(None) | Err(ReadbackError::Busy) => {}
                        Err(error) => panic!("take retained frame: {error:?}"),
                    }
                    assert!(Instant::now() < deadline, "retained frame deadline");
                    thread::sleep(Duration::from_millis(10));
                }
            }
        }
        if phase == GpuRetainedPhase::Draining {
            registrations.clear();
        }
        if phase == GpuRetainedPhase::Drained {
            // Retire the old gates without ever admitting more than q drawing heads.
            draw.store(false, Ordering::Relaxed);
            wake_slots(&play, &probes);
            let deadline = Instant::now() + Duration::from_secs(60);
            while probes.iter().any(|h| h.mailbox.head().is_some()) {
                assert!(Instant::now() < deadline, "GPU detach deadline");
                thread::sleep(Duration::from_millis(50));
            }
            for texture in &textures {
                *texture.lock().unwrap() = None;
            }
            draw.store(true, Ordering::Relaxed);
            wake_slots(&play, &probes);
            wait_gpu();
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let texture = textures[0].lock().unwrap();
                if let Some(texture) = texture.as_ref() {
                    match texture.try_register_consumer() {
                        Err(ReadbackError::Limit) if n == cap => {
                            overflow_refused = true;
                            break;
                        }
                        Err(ReadbackError::Busy) => {}
                        Ok(registration) if n < cap => {
                            drop(registration);
                            break;
                        }
                        _ => panic!("q+1 retained registration did not fail closed"),
                    }
                }
                drop(texture);
                assert!(
                    Instant::now() < deadline,
                    "new GPU gate registration deadline"
                );
                thread::sleep(Duration::from_millis(10));
            }
            for frame in held.drain(..) {
                let token = frame.present_token().get();
                let deadline = Instant::now() + Duration::from_secs(10);
                let permit = ReadbackBudget::process()
                    .try_acquire(CaptureGeometry::new(765, 503), deadline)
                    .unwrap();
                let mut pending = match frame.try_begin_readback(ReadbackKind::ValidScene, permit) {
                    Ok(pending) => pending,
                    Err((error, _, _)) => panic!("held readback: {error:?}"),
                };
                loop {
                    if let Some(done) = pending.poll(Instant::now()).unwrap() {
                        let (bytes, actual_token, valid, geometry, permit) = done.into_parts();
                        assert_eq!(actual_token.get(), token);
                        assert!(valid);
                        assert_eq!(bytes.len(), 765 * 503 * 4);
                        let hash = bytes.iter().fold(0xcbf29ce484222325u64, |h, b| {
                            (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
                        });
                        readbacks.push(json!({"token":token,"valid_scene":valid,
                            "rgba_bytes":bytes.len(),"hash_fnv1a64":hash,
                            "width":geometry.width(),"height":geometry.height()}));
                        drop(permit);
                        break;
                    }
                    assert!(Instant::now() < deadline, "held readback deadline");
                    thread::sleep(Duration::from_millis(10));
                }
            }
            drop(register(&textures[0]));
        }
        let started = Instant::now();
        let (before, _, first) = capture_sample("gpu-retained", 0, started, &play, &probes, None);
        let counters_before = present::retained_present_stats();
        if phase == GpuRetainedPhase::Registered {
            for texture in &textures {
                registrations.push(register(texture));
            }
        }
        let tokens: Vec<_> = held.iter().map(|f| f.present_token().get()).collect();
        thread::sleep(Duration::from_secs(30));
        let (after, _, last) =
            capture_sample("gpu-retained", 1, started, &play, &probes, Some(&before));
        let counters_after = present::retained_present_stats();
        receipts.push(GpuRetainedReceipt {
            phase,
            expected_resources: n,
            cap,
            live_resources: counters_after.live_resources(),
            logical_bytes: counters_after.logical_bytes(),
            allocations_before: counters_before.allocations(),
            allocations_after: counters_after.allocations(),
            copies_before: counters_before.copies(),
            copies_after: counters_after.copies(),
            copied_bytes_before: counters_before.copied_bytes(),
            copied_bytes_after: counters_after.copied_bytes(),
            copy_encode_ns: (counters_after.copy_encode_time()
                - counters_before.copy_encode_time())
            .as_nanos(),
            present_submit_ns: (counters_after.present_submit_time()
                - counters_before.present_submit_time())
            .as_nanos(),
            overflow_refused: false,
            heads: before
                .iter()
                .zip(&after)
                .enumerate()
                .map(|(index, (before, after))| {
                    let identity = after.head_identity.expect("live GPU identity");
                    GpuRetainedHead {
                        slot: identity.slot,
                        generation: identity.generation,
                        connected: after.connected && after.ingame,
                        gpu: after.backend == Some(BackendKind::Gpu),
                        valid_scene: after.has_valid_scene,
                        fault: after.refusal.is_some()
                            || after
                                .output
                                .is_none_or(|o| o.fault.is_some() || o.kind != "texture"),
                        cpu_clean_bytes: after
                            .owner_cpu_retained_bytes
                            .expect("GPU CPU helper census"),
                        paints_before: before.frame_generation,
                        paints_after: after.frame_generation,
                        scenes_before: before.scene_consumes,
                        scenes_after: after.scene_consumes,
                        held_token_before: tokens.get(index).copied(),
                        held_token_after: held.get(index).map(|f| f.present_token().get()),
                    }
                })
                .collect(),
        });
        samples.push(json!({"phase":phase,"before":first,"after":last}));
    }
    for receipt in &mut receipts {
        receipt.overflow_refused = overflow_refused;
    }
    let failures: Vec<_> = receipts
        .iter()
        .filter_map(|r| {
            gpu_retained_verdict(r)
                .err()
                .map(|error| format!("{:?}: {error}", r.phase))
        })
        .collect();
    std::fs::write(
        output.join(format!("gpu-n{n}-retained.json")),
        serde_json::to_vec_pretty(&json!({
            "schema":"t44-gpu-retained/v1","host":required("T44_LAB_HOST"),
            "gpu":required("T44_GPU_IDENTITY"),"git_rev":required("M3_GIT_REV"),
            "receipts":receipts,"samples":samples,"readbacks":readbacks,"failures":failures,
            "timing_scope":"CPU copy encoding and queue submission; not GPU execution",
            "readback_scope":"separate staging allocation after held/draining measurements"
        }))
        .unwrap(),
    )
    .unwrap();
    for head in &probes {
        if let Some(arm) = play.arm(&head.name) {
            arm.request_logout();
        }
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    while play.statuses().iter().any(|s| s.ingame) {
        assert!(Instant::now() < deadline, "GPU logout deadline");
        thread::sleep(Duration::from_millis(50));
    }
    consumer.stop_and_join();
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(present::retained_present_stats().live_resources(), 0);
}

const REGION_ORIGIN: Tile = Tile {
    x: 2659,
    z: 3292,
    level: 0,
};
const REGION_DESTINATION: Tile = Tile {
    x: 2724,
    z: 3484,
    level: 0,
};
const SEED_SCENE_TICK_BUDGET: u32 = 150;
const SCENE_LOADING_TICK_BUDGET: u32 = 600;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RegionPhase {
    AwaitingLogin,
    SeedPending,
    AtOrigin,
    Walking,
    Arrived,
}

impl RegionPhase {
    fn label(self) -> &'static str {
        match self {
            Self::AwaitingLogin => "awaiting_login",
            Self::SeedPending => "seed_pending",
            Self::AtOrigin => "at_origin",
            Self::Walking => "walking",
            Self::Arrived => "arrived",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RegionClientSignature {
    tick: u32,
    tile: Option<(i32, i32, i32)>,
    scene_state: i32,
    base: (i32, i32),
    flag: (i32, i32),
    phase: RegionPhase,
}

struct RegionRun {
    snapshot: GameSnapshot,
    pump: Pump,
    phase: RegionPhase,
    seeded: bool,
    seed_tick: Option<u32>,
    origin_tick: Option<u32>,
    starting_base: Option<(i32, i32)>,
    origin_overlay_flag: Option<(i32, i32)>,
    route_start_tick: Option<u32>,
    route_start_tile: Option<(i32, i32, i32)>,
    route_tick_budget: Option<u32>,
    route_tick_cost: Option<f64>,
    route_description: Option<String>,
    route_crosses_zone_edge: bool,
    route_has_displacing_transport: bool,
    source_route_confirmed: bool,
    walk_arms: WalkArms,
    route_arm: Option<Arc<Mutex<WalkArm>>>,
    last_walk_tick: Option<u32>,
    movement_ticks: u32,
    scene_loading_ticks: u32,
    natural_loading_tick: Option<u32>,
    arrival_tick: Option<u32>,
    overlay_change_tick: Option<u32>,
    overlay_clear_tick: Option<u32>,
    last_signature: Option<RegionClientSignature>,
    failure: Option<String>,
}

impl RegionRun {
    fn new() -> Self {
        Self {
            snapshot: GameSnapshot::new(),
            pump: Pump::new(),
            phase: RegionPhase::AwaitingLogin,
            seeded: false,
            seed_tick: None,
            origin_tick: None,
            starting_base: None,
            origin_overlay_flag: None,
            route_start_tick: None,
            route_start_tile: None,
            route_tick_budget: None,
            route_tick_cost: None,
            route_description: None,
            route_crosses_zone_edge: false,
            route_has_displacing_transport: false,
            source_route_confirmed: false,
            walk_arms: WalkArms::default(),
            route_arm: None,
            last_walk_tick: None,
            movement_ticks: 0,
            scene_loading_ticks: 0,
            natural_loading_tick: None,
            arrival_tick: None,
            overlay_change_tick: None,
            overlay_clear_tick: None,
            last_signature: None,
            failure: None,
        }
    }
}

fn route_crosses_zone_edge(route: &Route) -> bool {
    route.legs.iter().any(|leg| match leg {
        Leg::Walk { tiles } => tiles.windows(2).any(|step| {
            step[0].x.div_euclid(8) != step[1].x.div_euclid(8)
                || step[0].z.div_euclid(8) != step[1].z.div_euclid(8)
        }),
        Leg::Transport { .. } => false,
    })
}

fn route_has_displacing_transport(route: &Route) -> bool {
    route
        .legs
        .iter()
        .any(|leg| matches!(leg, Leg::Transport { edge } if edge.to != edge.at))
}

fn route_movement_tick_budget(route: &Route) -> u32 {
    // nav-planner/router.rs:6-8,178-181 sets run pace to 0.5 ticks/tile.
    // Count each actual step at one observed tick and retain every transport
    // edge cost plus one action tick. Two ticks cover movement terminal slack;
    // scene loading and current-flag clearing are separately observed.
    let mut ticks = 2u64;
    for leg in &route.legs {
        match leg {
            Leg::Walk { tiles } => {
                ticks = ticks.saturating_add(tiles.len().saturating_sub(1) as u64);
            }
            Leg::Transport { edge } => {
                ticks = ticks
                    .saturating_add(edge.ticks.max(0) as u64)
                    .saturating_add(1);
            }
        }
    }
    u32::try_from(ticks).unwrap_or(u32::MAX)
}

fn append_region_client_event(
    run: &mut RegionRun,
    client: &Client,
    mailbox: &FrameBuf,
    trace: &Arc<Mutex<RegionTrace>>,
) {
    let tick = run.snapshot.tick();
    let tile = run.snapshot.tile();
    let base = (client.map_build_base_x, client.map_build_base_z);
    let flag = (client.minimap_flag_x, client.minimap_flag_z);
    let signature = RegionClientSignature {
        tick,
        tile,
        scene_state: client.scene_state,
        base,
        flag,
        phase: run.phase,
    };
    if run.last_signature == Some(signature) {
        return;
    }

    let mut reasons = Vec::new();
    if run.phase == RegionPhase::AtOrigin && tile == Some((REGION_ORIGIN.x, REGION_ORIGIN.z, 0)) {
        reasons.push("origin_scene_ready");
    }
    if run.phase == RegionPhase::Walking && client.scene_state == 1 {
        reasons.push("natural_scene_state_1");
        run.natural_loading_tick.get_or_insert(tick);
    }
    if run
        .starting_base
        .is_some_and(|starting_base| base != starting_base)
    {
        reasons.push("map_build_base_changed");
    }
    if run
        .last_signature
        .is_some_and(|previous| previous.flag.0 != flag.0)
    {
        if flag.0 != 0 {
            reasons.push("minimap_flag_changed");
            run.overlay_change_tick.get_or_insert(tick);
        } else if run.overlay_change_tick.is_some() {
            reasons.push("minimap_flag_cleared");
            run.overlay_clear_tick.get_or_insert(tick);
        }
    }
    if client.scene_state == 2
        && run.natural_loading_tick.is_some_and(|loading_tick| {
            tick.wrapping_sub(loading_tick) > 0
                && run
                    .starting_base
                    .is_some_and(|starting_base| base != starting_base)
        })
    {
        reasons.push("post_rebuild_scene_complete");
    }
    if run.phase == RegionPhase::Arrived
        && tile == Some((REGION_DESTINATION.x, REGION_DESTINATION.z, 0))
    {
        reasons.push("route_arrival");
    }

    let event = RegionClientEvent {
        tick,
        tile,
        scene_state: client.scene_state,
        base,
        centre_zone: (
            client.map_build_centre_zone_x,
            client.map_build_centre_zone_z,
        ),
        minimap_flag: flag,
        main_overlay_id: client.main_overlay_id,
        main_modal_id: client.main_modal_id,
        phase: run.phase.label(),
        reasons,
        expected_generation: mailbox.generation().saturating_add(1),
    };
    trace.lock().unwrap().client_events.push(event);
    run.last_signature = Some(signature);
}

fn source_walk_contains_door(route: &Route) -> bool {
    route.legs.iter().any(|leg| {
        matches!(leg, Leg::Transport { edge }
            if edge.loc_id == 1530
                && edge.dir == Some(DoorDir::W)
                && edge.to == edge.at)
    })
}

fn record_route_failure(run: &mut RegionRun, message: impl Into<String>) {
    if run.failure.is_none() {
        run.failure = Some(message.into());
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "The real region driver needs the client, mailbox, trace, route signal and nav identity"
)]
fn drive_region_client(
    client: &mut Client,
    username: &str,
    mailbox: &FrameBuf,
    region: &Arc<Mutex<RegionRun>>,
    trace: &Arc<Mutex<RegionTrace>>,
    route_requested: &AtomicBool,
    world: &nav_planner::world::NavWorld,
    nav_digest: Digest,
) {
    client.set_draw(true);
    let mut run = region.lock().unwrap();
    let drain = run.pump.drain_client(client);
    host::publish_snapshot(&mut run.snapshot, client, drain);
    let tick = run.snapshot.tick();

    if !run.seeded && run.snapshot.ingame() && run.snapshot.attached() && client.scene_state == 2 {
        // The only fixture mutation is this account's own initial position.
        interact::seed_at(
            client,
            REGION_ORIGIN.level,
            REGION_ORIGIN.x,
            REGION_ORIGIN.z,
        );
        run.seeded = true;
        run.seed_tick = Some(tick);
        run.phase = RegionPhase::SeedPending;
    }

    if run.phase == RegionPhase::SeedPending {
        if run
            .seed_tick
            .is_some_and(|seed_tick| tick.wrapping_sub(seed_tick) > SEED_SCENE_TICK_BUDGET)
        {
            let observed_tile = run.snapshot.tile();
            record_route_failure(
                &mut run,
                format!(
                    "seeded origin not observed within {SEED_SCENE_TICK_BUDGET} server ticks: {observed_tile:?}"
                ),
            );
        } else if client.scene_state == 2
            && run.snapshot.tile() == Some((REGION_ORIGIN.x, REGION_ORIGIN.z, REGION_ORIGIN.level))
        {
            run.phase = RegionPhase::AtOrigin;
            run.origin_tick = Some(tick);
            run.starting_base = Some((client.map_build_base_x, client.map_build_base_z));
            run.origin_overlay_flag = Some((client.minimap_flag_x, client.minimap_flag_z));
        }
    }

    if run.phase == RegionPhase::AtOrigin
        && route_requested.load(Ordering::Acquire)
        && client.scene_state == 2
        && run.snapshot.tile() == Some((REGION_ORIGIN.x, REGION_ORIGIN.z, REGION_ORIGIN.level))
    {
        let context = MapContext {
            focus: Some(FocusToken::capture(&SlotArm::new(1, false))),
            nav: nav_digest,
            overlay: None,
            generation: 1,
        };
        let mut model = MapModel::default();
        model.bind(context);
        model.select_tile(world, REGION_DESTINATION);
        let command = match model.confirm(
            ActionKind::Walk,
            &context,
            Some(REGION_ORIGIN),
            FindOptions::default(),
        ) {
            Ok(command) => command,
            Err(error) => {
                record_route_failure(&mut run, format!("confirm source WalkTo route: {error:?}"));
                append_region_client_event(&mut run, client, mailbox, trace);
                return;
            }
        };
        let state = WorldState::from_snapshot(&run.snapshot);
        let admission = host_play::admission::Admission::manual(
            command.options(),
            host_play::admission::capture(
                &run.snapshot,
                state.map_members,
                Default::default(),
                false,
            ),
            0,
            host_play::WalkGlobals::default(),
        );
        let route = match command.walk_on(
            world,
            &context,
            username,
            &state,
            &BankRows::default(),
            admission,
            &run.walk_arms,
        ) {
            Ok(route) => route,
            Err(error) => {
                record_route_failure(&mut run, format!("plan source WalkTo route: {error:?}"));
                append_region_client_event(&mut run, client, mailbox, trace);
                return;
            }
        };
        let route_arm = run.walk_arms.lock().unwrap().get(username).cloned();
        let Some(route_arm) = route_arm else {
            record_route_failure(&mut run, "WalkTo did not publish its production WalkArm");
            append_region_client_event(&mut run, client, mailbox, trace);
            return;
        };
        run.route_description = Some(format!("{:?}", route.route.legs));
        run.route_tick_cost = Some(route.route.ticks);
        run.route_tick_budget = Some(route_movement_tick_budget(&route.route));
        run.route_crosses_zone_edge = route_crosses_zone_edge(&route.route);
        run.route_has_displacing_transport = route_has_displacing_transport(&route.route);
        run.source_route_confirmed = source_walk_contains_door(&route.route);
        run.route_arm = Some(route_arm);
        run.route_start_tick = Some(tick);
        run.route_start_tile = run.snapshot.tile();
        run.last_walk_tick = Some(tick);
        run.phase = RegionPhase::Walking;
        if !run.starting_base.is_some_and(|base| {
            outside_rebuild_window(
                base,
                (
                    REGION_DESTINATION.x,
                    REGION_DESTINATION.z,
                    REGION_DESTINATION.level,
                ),
            )
        }) {
            record_route_failure(
                &mut run,
                "source route endpoint does not leave the engine's REBUILD_NORMAL window",
            );
        }
        if !run.source_route_confirmed {
            record_route_failure(
                &mut run,
                "planned route no longer matches the source-backed Ardougne door walk",
            );
        }
        if !run.route_crosses_zone_edge {
            record_route_failure(
                &mut run,
                "planned source route does not cross an 8x8 scene-zone edge",
            );
        }
        if run.route_has_displacing_transport {
            record_route_failure(
                &mut run,
                "source route displaces the player through a transport instead of crossing only by walking",
            );
        }
    }

    if run.phase == RegionPhase::Walking && run.last_walk_tick != Some(tick) {
        if client.scene_state == 1 {
            run.scene_loading_ticks = run.scene_loading_ticks.saturating_add(1);
            if run.scene_loading_ticks > SCENE_LOADING_TICK_BUDGET {
                record_route_failure(
                    &mut run,
                    format!(
                        "natural check_scene exceeded {SCENE_LOADING_TICK_BUDGET} observed ticks"
                    ),
                );
            }
        } else if client.scene_state == 2 {
            run.movement_ticks = run.movement_ticks.saturating_add(1);
            let tick_budget = run.route_tick_budget.unwrap_or(0);
            if run.movement_ticks > tick_budget {
                record_route_failure(
                    &mut run,
                    format!("WalkTo exceeded its source-derived {tick_budget}-tick movement bound"),
                );
            } else {
                let route_arm = run.route_arm.clone();
                if let (Some(here), Some(route_arm)) = (run.snapshot.tile(), route_arm) {
                    let world_state = WorldState::from_snapshot(&run.snapshot);
                    let mut arm = route_arm.lock().unwrap();
                    host_play::step_walk_arm_follow(
                        client,
                        &run.snapshot,
                        &mut arm,
                        Some(world),
                        here,
                        world_state.map_members,
                        Some(username),
                    );
                    if arm.route.is_none() {
                        if here
                            == (
                                REGION_DESTINATION.x,
                                REGION_DESTINATION.z,
                                REGION_DESTINATION.level,
                            )
                        {
                            run.phase = RegionPhase::Arrived;
                            run.arrival_tick = Some(tick);
                        } else {
                            record_route_failure(
                                &mut run,
                                format!("production WalkTo terminated before arrival at {here:?}"),
                            );
                        }
                    }
                }
            }
        }
        run.last_walk_tick = Some(tick);
    }

    // draw.rs:836-840 clears the marker when the local player reaches it;
    // this observer runs before paint, so allow the next two observed ticks.

    if run.phase == RegionPhase::Arrived
        && run.overlay_clear_tick.is_none()
        && run
            .arrival_tick
            .is_some_and(|arrival_tick| tick.wrapping_sub(arrival_tick) > 2)
    {
        record_route_failure(
            &mut run,
            "the current minimap destination flag did not clear within two observed ticks of arrival",
        );
    }

    append_region_client_event(&mut run, client, mailbox, trace);
    if run.phase == RegionPhase::Arrived && run.natural_loading_tick.is_none() {
        record_route_failure(
            &mut run,
            "production arrival completed without an observed natural scene_state=1 loading transition",
        );
    }
    if run.phase == RegionPhase::Arrived
        && run.starting_base.is_some_and(|starting_base| {
            starting_base == (client.map_build_base_x, client.map_build_base_z)
        })
    {
        record_route_failure(
            &mut run,
            "production arrival completed without a real map-build base change",
        );
    }
}

fn frame_for_reason(trace: &RegionTrace, reason: &'static str) -> Option<RegionFrameEvidence> {
    trace
        .frames
        .iter()
        .find(|frame| {
            frame.event_ids.iter().any(|id| {
                trace
                    .client_events
                    .get(*id)
                    .is_some_and(|event| event.reasons.contains(&reason))
            })
        })
        .cloned()
}

fn rebuild_base_matches_centre_zone(event: &RegionClientEvent) -> bool {
    event.base == ((event.centre_zone.0 - 6) * 8, (event.centre_zone.1 - 6) * 8)
}

fn outside_rebuild_window(base: (i32, i32), tile: (i32, i32, i32)) -> bool {
    // Engine entity/BuildArea.ts:61-67: [originZone-4, originZone+5).
    // The client's base is (originZone-6)*8, so the relative bounds are 16..88.
    tile.0 < base.0 + 16 || tile.0 >= base.0 + 88 || tile.1 < base.1 + 16 || tile.1 >= base.1 + 88
}

#[test]
fn region_crossing_uses_engine_reload_window_not_each_eight_tile_zone() {
    let base = (2608, 3240);
    assert!(!outside_rebuild_window(base, (2624, 3256, 0)));
    assert!(!outside_rebuild_window(base, (2695, 3327, 0)));
    assert!(!outside_rebuild_window(base, (2661, 3301, 0)));
    assert!(outside_rebuild_window(base, (2623, 3256, 0)));
    assert!(outside_rebuild_window(base, (2696, 3327, 0)));
    assert!(outside_rebuild_window(base, (2659, 3255, 0)));
    assert!(outside_rebuild_window(base, (2660, 3328, 0)));
    assert!(outside_rebuild_window(
        base,
        (
            REGION_DESTINATION.x,
            REGION_DESTINATION.z,
            REGION_DESTINATION.level
        )
    ));
}

fn genuine_region_rebuild_seen(
    trace: &RegionTrace,
    starting_base: (i32, i32),
    planned_zone_crossing: bool,
    route_start: Option<(i32, i32, i32)>,
    destination: (i32, i32, i32),
) -> bool {
    let Some(route_start) = route_start else {
        return false;
    };
    let initial_scene_matches_base = trace.client_events.iter().any(|event| {
        event.phase == RegionPhase::Walking.label()
            && event.tile == Some(route_start)
            && event.base == starting_base
            && rebuild_base_matches_centre_zone(event)
    });
    let actual_zone_crossing = trace.client_events.iter().any(|event| {
        event.phase == RegionPhase::Walking.label()
            && event
                .tile
                .is_some_and(|tile| outside_rebuild_window(starting_base, tile))
    });
    let Some(loading) = trace.client_events.iter().find(|event| {
        event.phase == RegionPhase::Walking.label()
            && event.scene_state == 1
            && event.base != starting_base
            && rebuild_base_matches_centre_zone(event)
            && event.reasons.contains(&"natural_scene_state_1")
    }) else {
        return false;
    };
    planned_zone_crossing
        && initial_scene_matches_base
        && actual_zone_crossing
        && trace.client_events.iter().any(|event| {
            event.scene_state == 2
                && event.phase == RegionPhase::Arrived.label()
                && event.base != starting_base
                && rebuild_base_matches_centre_zone(event)
                && event.tile == Some(destination)
                && event.tick.wrapping_sub(loading.tick) > 0
                && event.reasons.contains(&"post_rebuild_scene_complete")
        })
}

#[test]
fn region_transition_oracle_rejects_a_paint_only_loading_state() {
    let base = (3280, 3240);
    let mut trace = RegionTrace {
        client_events: vec![
            RegionClientEvent {
                tick: 10,
                tile: Some((2659, 3292, 0)),
                scene_state: 2,
                base,
                centre_zone: (338, 418),
                minimap_flag: (0, 0),
                main_overlay_id: -1,
                main_modal_id: -1,
                phase: "walking",
                reasons: Vec::new(),
                expected_generation: 1,
            },
            RegionClientEvent {
                tick: 11,
                tile: Some((2659, 3292, 0)),
                scene_state: 1,
                base,
                centre_zone: (338, 418),
                minimap_flag: (0, 0),
                main_overlay_id: -1,
                main_modal_id: -1,
                phase: "walking",
                reasons: vec!["natural_scene_state_1"],
                expected_generation: 2,
            },
            RegionClientEvent {
                tick: 12,
                tile: Some((2661, 3301, 0)),
                scene_state: 2,
                base,
                centre_zone: (338, 418),
                minimap_flag: (0, 0),
                main_overlay_id: -1,
                main_modal_id: -1,
                phase: "arrived",
                reasons: vec!["post_rebuild_scene_complete"],
                expected_generation: 3,
            },
        ],
        frames: Vec::new(),
    };
    let route_start = Some((2659, 3292, 0));
    let destination = (2661, 3301, 0);
    assert!(!genuine_region_rebuild_seen(
        &trace,
        base,
        true,
        route_start,
        destination,
    ));

    let initial_base = (2608, 3240);
    let rebuilt_base = (2608, 3248);
    trace.client_events[0].base = initial_base;
    trace.client_events[0].centre_zone = (332, 411);
    trace.client_events[1].tile = Some((2660, 3296, 0));
    trace.client_events[1].base = rebuilt_base;
    trace.client_events[1].centre_zone = (332, 412);
    trace.client_events[2].base = rebuilt_base;
    trace.client_events[2].centre_zone = (332, 412);
    assert!(!genuine_region_rebuild_seen(
        &trace,
        initial_base,
        true,
        route_start,
        destination,
    ));

    // A source-valid rebuild first occurs at z=3328, not at the zone edge 3296.
    trace.client_events[1].tile = Some((2660, 3328, 0));
    trace.client_events[1].base = (2608, 3280);
    trace.client_events[1].centre_zone = (332, 416);
    let destination = (2724, 3484, 0);
    trace.client_events[2].tile = Some(destination);
    trace.client_events[2].base = (2672, 3432);
    trace.client_events[2].centre_zone = (340, 435);
    assert!(genuine_region_rebuild_seen(
        &trace,
        initial_base,
        true,
        route_start,
        destination,
    ));
}

#[test]
#[ignore = "LIVE=1 Engine L CPU real scene-region crossing; explicit drawing budgets required"]
fn real_region_crossing_draws_loading_splash_and_current_overlays() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_CPU").as_deref(), Ok("1"));
    let evidence_path = PathBuf::from(required("T44_REGION_CROSS_OUT"));
    if let Some(parent) = evidence_path.parent() {
        std::fs::create_dir_all(parent).expect("create region-crossing evidence directory");
    }
    client::profiling::enable();
    std::sync::LazyLock::force(&EPOCH);
    configure_drawing_fixture();

    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(env_or("BOT_GAME_PORT", 47594u16)),
        http_port: Some(env_or("BOT_HTTP_PORT", 3080u16)),
        engine_dir: Some(required("ENGINE_DIR").into()),
        nav_pack: Some(required("NAV_PACK").into()),
        ..ProfileOptions::default()
    };
    let profile = options
        .resolve(None)
        .expect("resolve local Engine L profile")
        .bind_runtime()
        .expect("bind local Engine L profile");
    let template = SharedClientTemplate::load(Arc::clone(&profile)).expect("load real profile");
    let world = template.world().expect("load the profile's real nav pack");
    let nav_digest = Digest::from_hex(
        &profile
            .nav_identity()
            .expect("profile nav identity")
            .nav_sha256,
    )
    .expect("parse profile nav digest");

    let name = mint_live_names(1).pop().expect("one account name");
    let input = SlotInput::new();
    input.set_prefer_cpu(true);
    input.set_render_focused(true);
    input.set_full_rate(true);
    let mailbox = FrameBuf::new();
    let loading = Arc::new(AtomicBool::new(false));
    let loading_paints = Arc::new(AtomicU64::new(0));
    let control = PaintControl {
        loading,
        loading_paints: Arc::clone(&loading_paints),
    };
    let trace = Arc::new(Mutex::new(RegionTrace::default()));
    let output = Arc::new(OutputProbe::for_region_trace(Arc::clone(&trace)));
    let region = Arc::new(Mutex::new(RegionRun::new()));
    let route_requested = Arc::new(AtomicBool::new(false));

    let frame_region = Arc::clone(&region);
    let frame_trace = Arc::clone(&trace);
    let frame_route_requested = Arc::clone(&route_requested);
    let frame_mailbox = Arc::clone(&mailbox);
    let frame_world = Arc::clone(&world);
    let frame_name = name.clone();
    let mut play = run_with_template(
        template,
        false,
        Vec::new(),
        |_| (None, None),
        move |client, username, _| {
            if username == frame_name {
                drive_region_client(
                    client,
                    username,
                    &frame_mailbox,
                    &frame_region,
                    &frame_trace,
                    &frame_route_requested,
                    &frame_world,
                    nav_digest,
                );
            }
        },
    )
    .expect("create one real Play");
    play.set_render_test_control(&name, Arc::new(control))
        .expect("bind non-forcing render observer before slot spawn");
    play.try_spawn_slot(
        live_profiles(std::slice::from_ref(&name))
            .into_iter()
            .next()
            .expect("one real profile"),
        Some(Arc::clone(&input)),
        Some(Arc::clone(&mailbox)),
        None,
    )
    .expect("spawn one real CPU slot");

    let stop = Arc::new(AtomicBool::new(false));
    let consumer = {
        let thread_stop = Arc::clone(&stop);
        let consumer_mailbox = Arc::clone(&mailbox);
        let consumer_output = Arc::clone(&output);
        let join = thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                if let Some(frame) = consumer_mailbox.take() {
                    consumer_output.observe(&consumer_mailbox, &frame);
                }
                render_head::pump(Instant::now());
                thread::sleep(CONSUMER_PERIOD);
            }
        });
        ConsumerGuard {
            stop: Arc::clone(&stop),
            join: Some(join),
        }
    };

    // check_scene notes its 360 s hang diagnostic at client.rs:11981-11984.
    // At Engine L's 600 ms tick, 600 observed loading ticks span that bound;
    // the broader wall deadline only detects a harness that stops callbacks.
    let harness_deadline = Instant::now() + Duration::from_secs(420);
    loop {
        let (phase, failure, tick) = {
            let run = region.lock().unwrap();
            (run.phase, run.failure.clone(), run.snapshot.tick())
        };
        if let Some(failure) = failure {
            panic!("region-crossing setup failed at tick {tick}: {failure}");
        }
        let origin_frame = {
            let trace = trace.lock().unwrap();
            frame_for_reason(&trace, "origin_scene_ready")
        };
        if phase == RegionPhase::AtOrigin
            && origin_frame.is_some_and(|frame| {
                frame.kind == "pixmap"
                    && frame.has_valid_scene == Some(true)
                    && frame.pixel_hash_fnv1a64.is_some()
            })
        {
            break;
        }
        assert!(
            Instant::now() < harness_deadline,
            "slot harness stopped before a valid origin scene; phase={phase:?}, tick={tick}"
        );
        thread::sleep(Duration::from_millis(10));
    }

    route_requested.store(true, Ordering::Release);
    play.wake(&name);
    loop {
        let (
            phase,
            failure,
            tick,
            start_base,
            start_tile,
            route_crosses,
            source_route,
            arrival_tick,
            overlay_change_tick,
            overlay_clear_tick,
            movement_ticks,
            scene_loading_ticks,
        ) = {
            let run = region.lock().unwrap();
            (
                run.phase,
                run.failure.clone(),
                run.snapshot.tick(),
                run.starting_base,
                run.route_start_tile,
                run.route_crosses_zone_edge,
                run.source_route_confirmed,
                run.arrival_tick,
                run.overlay_change_tick,
                run.overlay_clear_tick,
                run.movement_ticks,
                run.scene_loading_ticks,
            )
        };
        if let Some(failure) = failure {
            panic!("region-crossing route failed at tick {tick}: {failure}");
        }
        let trace_snapshot = trace.lock().unwrap();
        let scene_complete_frame = frame_for_reason(&trace_snapshot, "post_rebuild_scene_complete");
        let loading_frame = frame_for_reason(&trace_snapshot, "natural_scene_state_1");
        let flag_frame = frame_for_reason(&trace_snapshot, "minimap_flag_changed");
        let clear_frame = frame_for_reason(&trace_snapshot, "minimap_flag_cleared");
        let has_genuine_rebuild = start_base.is_some_and(|base| {
            genuine_region_rebuild_seen(
                &trace_snapshot,
                base,
                route_crosses,
                start_tile,
                (
                    REGION_DESTINATION.x,
                    REGION_DESTINATION.z,
                    REGION_DESTINATION.level,
                ),
            )
        });
        let complete = phase == RegionPhase::Arrived
            && arrival_tick.is_some()
            && overlay_change_tick.is_some()
            && overlay_clear_tick.is_some()
            && has_genuine_rebuild
            && source_route
            && loading_frame.is_some_and(|frame| {
                frame.kind == "pixmap"
                    && frame.scene_progress.as_deref() == Some("Retained")
                    && frame.has_valid_scene == Some(true)
                    && frame.pixel_hash_fnv1a64.is_some()
            })
            && scene_complete_frame.is_some_and(|frame| {
                frame.kind == "pixmap"
                    && frame.scene_progress.as_deref() == Some("Rebuilt")
                    && frame.has_valid_scene == Some(true)
                    && frame.pixel_hash_fnv1a64.is_some()
            })
            && flag_frame.is_some_and(|frame| frame.pixel_hash_fnv1a64.is_some())
            && clear_frame.is_some_and(|frame| frame.pixel_hash_fnv1a64.is_some());
        drop(trace_snapshot);
        if complete {
            break;
        }
        let movement_budget = region.lock().unwrap().route_tick_budget.unwrap_or_default();
        assert!(
            movement_ticks <= movement_budget,
            "region crossing exceeded the source-derived movement tick budget"
        );
        assert!(
            scene_loading_ticks <= SCENE_LOADING_TICK_BUDGET,
            "natural scene loading exceeded its source-backed observed-tick budget"
        );
        assert!(
            Instant::now() < harness_deadline,
            "slot harness stopped before real region crossing completed; phase={phase:?}, tick={tick}"
        );
        thread::sleep(Duration::from_millis(10));
    }

    let run_snapshot = {
        let run = region.lock().unwrap();
        assert_eq!(run.seeded as u32, 1, "the account was seeded exactly once");
        assert_eq!(
            run.origin_overlay_flag.map(|flag| flag.0),
            Some(0),
            "the seeded scene begins without a minimap destination flag"
        );
        assert!(run.route_description.is_some(), "record planned route");
        assert!(
            run.route_crosses_zone_edge,
            "planned route crosses a real scene-zone edge"
        );
        assert!(
            run.source_route_confirmed,
            "the nav-pack route retains its source door"
        );
        assert!(
            !run.route_has_displacing_transport,
            "the source route crosses its zone edge without teleporting"
        );
        json!({
            "seed_calls": run.seeded as u32,
            "origin": [REGION_ORIGIN.x, REGION_ORIGIN.z, REGION_ORIGIN.level],
            "destination": [REGION_DESTINATION.x, REGION_DESTINATION.z, REGION_DESTINATION.level],
            "origin_tick": run.origin_tick,
            "route_start_tick": run.route_start_tick,
            "arrival_tick": run.arrival_tick,
            "overlay_flag_changed_tick": run.overlay_change_tick,
            "overlay_flag_cleared_tick": run.overlay_clear_tick,
            "starting_base": run.starting_base,
            "route_tick_cost": run.route_tick_cost,
            "route_movement_tick_budget": run.route_tick_budget,
            "route_movement_ticks": run.movement_ticks,
            "scene_loading_ticks": run.scene_loading_ticks,
            "planned_route_crosses_scene_zone_edge": run.route_crosses_zone_edge,
            "rebuild_boundary_source": "engine/src/engine/entity/BuildArea.ts:57-67",
            "source_route_has_ardougne_west_door_1530": run.source_route_confirmed,
            "route_has_displacing_transport": run.route_has_displacing_transport,
            "walk_legs": run.route_description,
        })
    };

    let (loading_frame, origin_frame, flag_frame, clear_frame, scene_complete_frame, trace_json) = {
        let trace = trace.lock().unwrap();
        (
            frame_for_reason(&trace, "natural_scene_state_1")
                .expect("captured natural loading frame"),
            frame_for_reason(&trace, "origin_scene_ready").expect("captured valid origin scene"),
            frame_for_reason(&trace, "minimap_flag_changed")
                .expect("captured changed current overlay"),
            frame_for_reason(&trace, "minimap_flag_cleared")
                .expect("captured cleared current overlay"),
            frame_for_reason(&trace, "post_rebuild_scene_complete")
                .expect("captured rebuilt scene"),
            region_trace_json(&trace),
        )
    };
    assert_ne!(
        loading_frame.splash_band_hash_fnv1a64, origin_frame.splash_band_hash_fnv1a64,
        "the naturally dispatched loading splash changes its source-backed text band"
    );
    assert_ne!(
        flag_frame.minimap_band_hash_fnv1a64, origin_frame.minimap_band_hash_fnv1a64,
        "the real WalkTo destination flag changes current minimap pixels"
    );
    assert_ne!(
        clear_frame.minimap_band_hash_fnv1a64, flag_frame.minimap_band_hash_fnv1a64,
        "the production destination flag clearing changes current minimap pixels again"
    );
    assert_ne!(
        scene_complete_frame.pixel_hash_fnv1a64, loading_frame.pixel_hash_fnv1a64,
        "the complete post-crossing scene publishes distinct pixels"
    );
    assert_eq!(
        loading_paints.load(Ordering::Relaxed),
        0,
        "the live cell never enabled the deterministic loading-paint override"
    );

    if let Some(arm) = play.arm(&name) {
        arm.request_logout();
    }
    let logout_deadline = Instant::now() + Duration::from_secs(30);
    while play.statuses().iter().any(|status| status.ingame) && Instant::now() < logout_deadline {
        thread::sleep(Duration::from_millis(10));
    }
    consumer.stop_and_join();
    assert!(
        !play.statuses().iter().any(|status| status.ingame),
        "the live region-crossing account logged out"
    );

    let receipt = json!({
        "schema": "t44-real-region-crossing/v1",
        "qualification": "one-account real Play CPU slot; production nav-pack walk; natural REBUILD_NORMAL only",
        "engine_profile": "local-289 Engine L",
        "game_port": env_or("BOT_GAME_PORT", 47594u16),
        "http_port": env_or("BOT_HTTP_PORT", 3080u16),
        "route_source": "crates/host-play/tests/nav_origin_live.rs:123-221",
        "rebuild_source": "vendor/fr-client-rust/crates/client/src/client/client.rs:8789-8809,11965-12041",
        "loading_splash_source": "vendor/fr-client-rust/crates/client/src/render/draw.rs:4167-4190",
        "overlay_source": "vendor/fr-client-rust/crates/client/src/client/client.rs:5204-5205,8485-8488; vendor/fr-client-rust/crates/client/src/render/draw.rs:838-840,3880-3892",
        "overlay_clear_observation": "the current client minimap flag returned to x=0 after production arrival; local arrival-draw and UNSET_MAP_FLAG clears are not distinguished",
        "paint_only_loading_override_enabled": false,
        "run": run_snapshot,
        "pixel_evidence": {
            "origin_frame_generation": origin_frame.generation,
            "origin_pixel_hash_fnv1a64": origin_frame.pixel_hash_fnv1a64,
            "loading_frame_generation": loading_frame.generation,
            "loading_pixel_hash_fnv1a64": loading_frame.pixel_hash_fnv1a64,
            "loading_splash_band_hash_fnv1a64": loading_frame.splash_band_hash_fnv1a64,
            "flag_frame_generation": flag_frame.generation,
            "flag_minimap_band_hash_fnv1a64": flag_frame.minimap_band_hash_fnv1a64,
            "cleared_flag_frame_generation": clear_frame.generation,
            "cleared_minimap_band_hash_fnv1a64": clear_frame.minimap_band_hash_fnv1a64,
            "rebuilt_scene_frame_generation": scene_complete_frame.generation,
            "rebuilt_scene_pixel_hash_fnv1a64": scene_complete_frame.pixel_hash_fnv1a64,
        },
        "trace": trace_json,
    });
    std::fs::write(
        &evidence_path,
        serde_json::to_vec_pretty(&receipt).expect("serialize region-crossing evidence"),
    )
    .expect("write region-crossing evidence");
    eprintln!(
        "t44 real region-crossing evidence: {}",
        evidence_path.display()
    );
}
