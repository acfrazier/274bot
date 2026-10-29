//! Drawing-only allocation evidence. M3 timing never installs this allocator.
#![cfg(feature = "draw-memory-profile")]

use client::render::backend::{cpu::CpuBackend, RenderBackend};
use host_play::memory::{rust_allocator_counts, CountingAllocator};

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

use client::render::backend::{BackendKind, FrameOutput, SceneProgress};
use client::render::budget::RenderBytes;
use host::render_head::test_hooks::{RenderTestControl, RenderTestEvent};
use host::{render_head, FrameBuf, SlotInput};
use host_play::{
    current_resident_bytes, mint_live_names, run_with_template, Play, ProfileOptions,
    SharedClientTemplate,
};
use serde_json::{json, Value};
use vault::{Profile, ProfileSettings, RasterMode};

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

#[derive(Default)]
struct OutputProbe {
    latest: Mutex<Option<OutputObservation>>,
}

impl OutputProbe {
    fn observe(&self, mailbox: &FrameBuf, output: FrameOutput) {
        let (kind, width, height, pixels) = match &output {
            FrameOutput::PixMap(pixmap) => (
                "pixmap",
                Some(pixmap.width),
                Some(pixmap.height),
                Some(pixmap.pixels.len()),
            ),
            FrameOutput::Texture(texture) => (
                "texture",
                Some(texture.width as i32),
                Some(texture.height as i32),
                None,
            ),
            FrameOutput::Refused(_) => ("refused", None, None, None),
        };
        let fault = output
            .render_fault()
            .or_else(|| mailbox.render_fault())
            .map(|_| "render_fault");
        let sample = OutputObservation {
            kind,
            width,
            height,
            pixels,
            fresh: output.is_fresh(),
            fault,
            generation: mailbox.generation(),
        };
        *self.latest.lock().unwrap() = Some(sample);
    }

    fn latest(&self) -> Option<OutputObservation> {
        *self.latest.lock().unwrap()
    }
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
                if head.owner_bytes.is_none_or(|bytes| bytes.total() != 0) {
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
        "accounting_scope": "per-head backend: capture excludes shared host mesh; context_cpu includes shared Media",
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
        "gpu_vertex_live": bytes.gpu_vertex_live(),
        "gpu_vertex_retired": bytes.gpu_vertex_retired(),
        "gpu_nav_target": bytes.gpu_nav_target(),
        "gpu_retained_present_per_backend_gate_bytes": bytes.gpu_retained_present(),
        "gpu_upload_scratch": bytes.gpu_upload_scratch(),
        "context_cpu": bytes.context_cpu(),
        "context_gpu": bytes.context_gpu(),
        "scalar_state": bytes.scalar_state(),
        "backend_total_bytes": bytes.total(),
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
        &[live_without_backend_gate.clone()],
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
        Some(&[old.clone()])
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
        Some(&[old.clone()])
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
        Some(&[old.clone()])
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
        .bind()
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
                        output.observe(mailbox, frame);
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
