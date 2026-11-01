//! Per-slot render heads over the process render service (I-render wrap,
//! T4.2; D12 as amended by §23 §2).
//!
//! One process [`RenderService`] admits every head. A drawing slot attaches a
//! [`HeadLease`] carrying `(SlotKey, head_generation)` at its first paint
//! after draw turns on. It retires the lease when draw turns off, when the
//! head's backend or memory mode no longer matches the slot, and when the
//! slot loop that owns the head ends (logout, a title wait handing the title
//! to the pump, slot exit). A reattached head is a new generation over the
//! same `Client`: attach never reconnects (ADR 0001).
//!
//! The wrap keeps today's per-slot `Renderer`:
//! - a CPU head (`CpuPix3D`) rasterizes into its own buffers and takes no
//!   scene-workspace reservation;
//! - a GPU head holds a [`FrameLease`] for exactly its scene submission, the
//!   section the client's `GPU_SCENE_LOCK` serializes, through the client's
//!   scene-workspace hook. While another head holds the workspace it waits
//!   for its own grant, where it used to wait for that mutex.
//!
//! Retained limitation (wrap only): admission blocks and claims no fairness.
//! The process-wide `GPU_SCENE_LOCK` (`client::render::backend::gpu`) still
//! runs inside the leased section. T4.4 replaces both waits with nonblocking
//! fair admission.
//!
//! A head publishes frames only into its slot's output mailbox while it is
//! that mailbox's attached head ([`FrameBuf`] checks the identity), and a
//! GPU head publishes its retained scene only through its live frame lease.

use std::cell::RefCell;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use api::host_log;
use api::hostlog::{Category, Level};
use api::identity::{Deadline, Notice, Notifier, PublishSequence, SlotKey};
use client::client::Client;
use client::render::backend::{gpu::SceneSubmit, BackendKind, FrameOutput};
use client::render::Renderer;

use crate::render::{
    FrameLease, HeadIdentity, HeadLease, RenderError, RenderPumpLease, RenderService,
    RequestOutcome, ReservationIdentity, SceneHandle,
};
use crate::slot_io::FramePaintStatus;
use crate::FrameBuf;

#[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
mod fair;
#[cfg(feature = "render-test-hooks")]
pub use fair::test_hooks;
#[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
pub use fair::{
    configure_calibration, configure_calibration_limits, with_process_render_budget,
    CalibrationConfig, DueOpportunity, GrantLatency, HeadFairStats,
};

#[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
pub(crate) fn slot_budget_scope(
    head: Option<&Head>,
) -> Option<client::render::budget::RenderBudgetScope> {
    head.and_then(Head::budget_scope)
        .or_else(fair::process_budget_scope)
}

/// How long a granted GPU head may take to consume its grant. A waiting head
/// consumes within its wake-up latency, so this bounds only a head whose
/// thread stalls between grant and consume. Wrap-only and unqualified: M3
/// publishes the production `RENDER_GRANT_TTL` before T4.4.
pub const WRAP_GRANT_TTL: Duration = Duration::from_millis(100);

/// The longest a waiting GPU head sleeps before it polls the service again,
/// so a missed notification or a stalled grant costs at most this long.
const GRANT_WAIT_SLICE: Duration = Duration::from_millis(10);

/// The one process service, its independent pump registration and the
/// counters of its GPU scene submissions.
struct ProcessRender {
    service: RenderService,
    pump: RenderPumpLease,
    counters: WorkspaceCounters,
}

/// The process service, independent pump and GPU scene-submission counters.
fn build_process_render() -> ProcessRender {
    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    let calibration = fair::initialize();
    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    if calibration.is_some() {
        client::render::backend::gpu::set_scene_admission_hook(fair::admission);
        client::render::backend::cpu::set_cpu_host_overlay_hook(fair::host_active);
    }
    client::render::backend::gpu::set_scene_workspace_hook(scene_workspace);
    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    let ttl = calibration.map_or(WRAP_GRANT_TTL, |config| config.grant_ttl);
    #[cfg(not(any(feature = "render-diagnostics", feature = "render-test-hooks")))]
    let ttl = WRAP_GRANT_TTL;
    let service = RenderService::new(ttl).expect("the grant lifetime is positive and finite");
    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    if let Some(config) = calibration {
        service
            .set_consumed_quantum(config.scene_quantum)
            .expect("validated calibration quantum");
    }
    let pump = service
        .register_pump()
        .expect("a new service registers its one pump");
    ProcessRender {
        service,
        pump,
        counters: WorkspaceCounters::default(),
    }
}

static PROCESS: LazyLock<ProcessRender> = LazyLock::new(build_process_render);

/// Run the process render pump at `now` and return its next deadline. The
/// headed composition calls it from its own loop (the panel window), so a
/// grant whose head stopped consuming expires at its deadline even when no
/// slot asks for a frame.
pub fn pump(now: Instant) -> Option<Deadline> {
    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    {
        let entered_at = Instant::now();
        PROCESS.counters.record_pump(entered_at);
    }
    match PROCESS.service.pump(&PROCESS.pump, now) {
        Ok(deadline) => {
            #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
            if fair::enabled() {
                return fair::heartbeat(now, deadline);
            }
            deadline
        }
        Err(error) => {
            host_log!(Category::Lifecycle, Level::Warn, "render pump: {error}");
            None
        }
    }
}

/// What GPU scene submissions in this process have cost in workspace
/// admission so far: the evidence for the wrap's retained wait.
pub fn workspace_stats() -> WorkspaceStats {
    let mut stats = PROCESS.counters.stats();
    stats.faults = PROCESS.service.fault_stats();
    stats
}

/// Acknowledge an incident without clearing its totals or live ownership.
pub fn acknowledge_fault(sequence: u64) -> Result<(), RenderError> {
    PROCESS.service.acknowledge_fault(sequence)
}

/// Whether the provisional fairness calibration was explicitly configured.
/// Querying this does not initialize the process render service.
#[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
pub fn fair_enabled() -> bool {
    fair::enabled()
}

/// A snapshot of the process's GPU scene-workspace counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkspaceStats {
    leased: u64,
    waited: u64,
    wait_ns: u64,
    max_wait_ns: u64,
    unleased: u64,
    faults: crate::render::RenderFaultStats,
    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    busy: u64,
    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    pump_calls: u64,
    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    max_pump_interval_ns: u64,
}

impl WorkspaceStats {
    pub const fn faults(&self) -> crate::render::RenderFaultStats {
        self.faults
    }

    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    pub const fn busy(&self) -> u64 {
        self.busy
    }
    /// Process-lifetime count of calls to the central render pump.
    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    pub const fn pump_calls(self) -> u64 {
        self.pump_calls
    }

    /// Longest process-lifetime interval between central render pump entries.
    /// Measured with actual monotonic time, independently of caller time.
    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    pub const fn max_pump_interval(self) -> Duration {
        Duration::from_nanos(self.max_pump_interval_ns)
    }

    /// Scene submissions that ran under a frame lease.
    pub const fn leased(self) -> u64 {
        self.leased
    }

    /// Of the leased submissions, those that found the workspace held and
    /// waited for their own grant.
    pub const fn waited(self) -> u64 {
        self.waited
    }

    /// Total time leased submissions spent waiting for their grant.
    pub const fn wait(self) -> Duration {
        Duration::from_nanos(self.wait_ns)
    }

    /// The longest single wait for a grant.
    pub const fn max_wait(self) -> Duration {
        Duration::from_nanos(self.max_wait_ns)
    }

    /// Submissions that ran under the client's lock alone because their head
    /// had no grant (retired under the frame).
    pub const fn unleased(self) -> u64 {
        self.unleased
    }
}

#[derive(Default)]
struct WorkspaceCounters {
    leased: AtomicU64,
    waited: AtomicU64,
    wait_ns: AtomicU64,
    max_wait_ns: AtomicU64,
    unleased: AtomicU64,
    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    busy: AtomicU64,
    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    pump_calls: AtomicU64,
    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    last_pump_at: Mutex<Option<Instant>>,
    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    max_pump_interval_ns: AtomicU64,
}

impl WorkspaceCounters {
    fn leased(&self, wait: Option<Duration>) {
        self.leased.fetch_add(1, Ordering::Relaxed);
        if let Some(wait) = wait {
            let nanos = u64::try_from(wait.as_nanos()).unwrap_or(u64::MAX);
            self.waited.fetch_add(1, Ordering::Relaxed);
            self.wait_ns.fetch_add(nanos, Ordering::Relaxed);
            self.max_wait_ns.fetch_max(nanos, Ordering::Relaxed);
        }
    }

    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    fn record_pump(&self, entered_at: Instant) {
        self.pump_calls.fetch_add(1, Ordering::Relaxed);
        let previous = lock(&self.last_pump_at).replace(entered_at);
        if let Some(previous) = previous {
            let nanos = u64::try_from(entered_at.saturating_duration_since(previous).as_nanos())
                .unwrap_or(u64::MAX);
            self.max_pump_interval_ns
                .fetch_max(nanos, Ordering::Relaxed);
        }
    }

    fn unleased(&self) {
        self.unleased.fetch_add(1, Ordering::Relaxed);
    }

    fn stats(&self) -> WorkspaceStats {
        WorkspaceStats {
            leased: self.leased.load(Ordering::Relaxed),
            waited: self.waited.load(Ordering::Relaxed),
            wait_ns: self.wait_ns.load(Ordering::Relaxed),
            max_wait_ns: self.max_wait_ns.load(Ordering::Relaxed),
            unleased: self.unleased.load(Ordering::Relaxed),
            faults: crate::render::RenderFaultStats::default(),
            #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
            busy: self.busy.load(Ordering::Relaxed),
            #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
            pump_calls: self.pump_calls.load(Ordering::Relaxed),
            #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
            max_pump_interval_ns: self.max_pump_interval_ns.load(Ordering::Relaxed),
        }
    }
}

/// A slot worker's render envelope: the worker's [`SlotKey`] and its
/// single-consumer output mailbox. Composition mints the key once per worker
/// lifetime, so every head the worker attaches carries it.
#[derive(Clone)]
pub struct SlotRender {
    slot: SlotKey,
    output: Arc<FrameBuf>,
    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    wake: Arc<dyn Notifier>,
    #[cfg(feature = "render-test-hooks")]
    test_control: Option<Arc<dyn test_hooks::RenderTestControl>>,
}

impl SlotRender {
    /// The envelope for one slot worker.
    pub fn new(slot: SlotKey, output: Arc<FrameBuf>, _wake: Arc<dyn Notifier>) -> Self {
        Self {
            slot,
            output,
            #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
            wake: _wake,
            #[cfg(feature = "render-test-hooks")]
            test_control: fair::configured_test_control(),
        }
    }

    #[cfg(feature = "render-test-hooks")]
    pub fn with_test_control(mut self, control: Arc<dyn test_hooks::RenderTestControl>) -> Self {
        self.test_control = Some(control);
        self
    }

    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    pub fn fair_stats(&self) -> Option<HeadFairStats> {
        self.output.fair_stats()
    }

    #[cfg(feature = "render-test-hooks")]
    pub(crate) fn render_now(&self, fallback: Instant) -> Instant {
        if fair::enabled() {
            self.test_control
                .as_ref()
                .map_or(fallback, |control| control.now())
        } else {
            fallback
        }
    }

    #[cfg(feature = "render-test-hooks")]
    pub(crate) fn loading_paint(&self) -> bool {
        fair::enabled()
            && self
                .test_control
                .as_ref()
                .is_some_and(|control| control.loading_paint())
    }

    /// The slot worker lifetime every head of this slot carries.
    pub fn slot(&self) -> SlotKey {
        self.slot
    }

    /// The slot's single-consumer output mailbox.
    pub fn output(&self) -> &Arc<FrameBuf> {
        &self.output
    }
}

impl fmt::Debug for SlotRender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlotRender")
            .field("slot", &self.slot)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "render-diagnostics")]
struct BudgetObservationClock(Instant);

#[cfg(feature = "render-diagnostics")]
impl BudgetObservationClock {
    fn due(&mut self, now: Instant) -> bool {
        if now.saturating_duration_since(self.0) < std::time::Duration::from_secs(1) {
            return false;
        }
        self.0 = now;
        true
    }
}
/// One attached head: its service lease and the per-slot `Renderer` that
/// draws it.
pub(crate) struct Head {
    /// `None` only while a GPU frame lends the lease to the scene hook.
    lease: Option<HeadLease>,
    identity: HeadIdentity,
    wake: Arc<HeadWake>,
    output: Arc<FrameBuf>,
    /// GPU scene submissions this head has published as its retained scene.
    scenes: u64,
    renderer: Renderer,
    /// The build request, so a GPU↔CPU or memory-mode change on a live slot
    /// is a retire and reattach, not a restart.
    prefer_cpu: bool,
    lowmem: bool,
    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    fair: fair::FairHead,
    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    budget: Option<client::render::budget::RenderBudget>,
    #[cfg(feature = "render-diagnostics")]
    budget_observation_clock: BudgetObservationClock,
}

impl Head {
    /// Attach `render`'s slot to the process service, then build its backend.
    /// A refused attach builds nothing. From here on the slot's output
    /// mailbox accepts this head's frames.
    pub(crate) fn attach(
        render: &SlotRender,
        prefer_cpu: bool,
        lowmem: bool,
    ) -> Result<Self, RenderError> {
        #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
        let wake = Arc::new(HeadWake::new(Arc::clone(&render.wake)));
        #[cfg(not(any(feature = "render-diagnostics", feature = "render-test-hooks")))]
        let wake = Arc::new(HeadWake::default());
        let notifier: Arc<dyn Notifier> = wake.clone();
        let lease = PROCESS
            .service
            .attach(&PROCESS.pump, render.slot, notifier)?;
        let identity = lease.identity();
        #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
        let budget = fair::head_budget();
        #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
        let _budget_scope = budget
            .as_ref()
            .map(client::render::budget::RenderBudget::scope);
        // GPU-first (the host default): the head prefers the wgpu backend,
        // with `CpuBackend` as the fallback on wgpu init failure
        // (`Renderer::new_prefer` selects, `Renderer::backend_kind` reports);
        // `BOT_CPU=1` forces the CPU fidelity path. GPU set-up blocks on the
        // driver: not time the host kept us waiting.
        let renderer = crate::capacity::off_cpu_wait(|| Renderer::new_prefer(lowmem, !prefer_cpu));
        render.output.attach_head(identity);
        #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
        let fair = {
            let fair = fair::FairHead::new(identity, renderer.backend_kind());
            #[cfg(feature = "render-test-hooks")]
            let fair = fair.with_control(render.test_control.clone());
            fair
        };
        #[cfg(feature = "render-diagnostics")]
        emit_budget_samples(budget.as_ref(), identity, "initial");
        Ok(Self {
            lease: Some(lease),
            identity,
            wake,
            output: Arc::clone(&render.output),
            scenes: 0,
            renderer,
            prefer_cpu,
            lowmem,
            #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
            fair,
            #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
            budget,
            #[cfg(feature = "render-diagnostics")]
            budget_observation_clock: BudgetObservationClock(Instant::now()),
        })
    }

    /// The `(SlotKey, head_generation)` this head carries.
    #[cfg(test)]
    pub(crate) fn identity(&self) -> HeadIdentity {
        self.identity
    }

    /// The backend state behind this head (title flames, minimap).
    #[cfg(test)]
    pub(crate) fn renderer(&self) -> &Renderer {
        &self.renderer
    }

    /// Whether the head was built with the CPU request.
    pub(crate) fn prefer_cpu(&self) -> bool {
        self.prefer_cpu
    }

    /// The memory mode the head was built with.
    pub(crate) fn lowmem(&self) -> bool {
        self.lowmem
    }

    /// The backend the head actually got (a GPU request can fall back).
    pub(crate) fn backend_kind(&self) -> BackendKind {
        self.renderer.backend_kind()
    }

    /// Release retained GPU nav resources after the client input clears.
    pub(crate) fn release_nav_debug_overlay(&mut self) {
        self.renderer.release_nav_debug_overlay();
    }

    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    pub(crate) fn budget_scope(&self) -> Option<client::render::budget::RenderBudgetScope> {
        self.budget
            .as_ref()
            .map(client::render::budget::RenderBudget::scope)
    }

    #[cfg(feature = "render-test-hooks")]
    pub(crate) fn observe_client_cycle(&self, client: &Client) {
        if fair::enabled() {
            self.fair.observe_client_cycle(
                self.identity.slot(),
                self.renderer.backend_kind(),
                client.loop_cycle,
                client.ingame && client.scene_state == 2 && self.renderer.has_valid_scene(),
            );
        }
    }

    /// Paint one frame. A GPU head lends its lease to this thread's scene
    /// hook for the frame, so its scene submission runs under a frame lease.
    #[inline]
    pub(crate) fn paint(&mut self, client: &mut Client) -> FrameOutput {
        #[cfg(feature = "draw-memory-case")]
        {
            crate::draw_memory_case::paint(client, self.identity, |client| {
                let output = self.paint_without_memory_case(client);
                let census = self.renderer.memory_case_census(&output);
                (output, census)
            })
        }
        #[cfg(not(feature = "draw-memory-case"))]
        self.paint_without_memory_case(client)
    }

    fn paint_without_memory_case(&mut self, client: &mut Client) -> FrameOutput {
        #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
        let _budget_scope = self
            .budget
            .as_ref()
            .map(client::render::budget::RenderBudget::scope);
        let frame = {
            #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
            if fair::enabled() {
                fair::paint(self, client)
            } else {
                self.paint_legacy(client)
            }
            #[cfg(not(any(feature = "render-diagnostics", feature = "render-test-hooks")))]
            self.paint_legacy(client)
        };
        #[cfg(feature = "draw-memory-profile")]
        if let Some(request) = self.output.render_bytes_request(self.identity) {
            let output_pixels = match &frame {
                FrameOutput::PixMap(pixmap) => {
                    pixmap.pixels.capacity() * std::mem::size_of::<i32>()
                }
                _ => self.output.render_output_storage_bytes(),
            };
            self.output.publish_render_bytes_sample(
                self.identity,
                request,
                self.renderer.render_bytes().with_host_storage(
                    output_pixels,
                    client
                        .nav_debug_paint()
                        .map_or(0, |paint| paint.retained_bytes()),
                    client.world.render_storage_bytes(),
                ),
            );
        }
        #[cfg(feature = "render-diagnostics")]
        if self.budget_observation_clock.due(Instant::now()) {
            emit_budget_samples(self.budget.as_ref(), self.identity, "periodic");
        }
        frame
    }

    fn paint_legacy(&mut self, client: &mut Client) -> FrameOutput {
        if self.renderer.backend_kind() != BackendKind::Gpu {
            return self.renderer.mainredraw(client);
        }
        let _lent = LentHead::lend(&mut self.lease, &mut self.scenes, &self.wake);
        self.renderer.mainredraw(client)
    }

    fn publication_status(&self) -> FramePaintStatus {
        #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
        let recovered_fault = if fair::enabled() {
            self.renderer.render_fault()
        } else {
            None
        };
        #[cfg(not(any(feature = "render-diagnostics", feature = "render-test-hooks")))]
        let recovered_fault = None;
        FramePaintStatus::new(self.renderer.has_valid_scene(), recovered_fault)
    }

    /// Publish a painted frame into the slot's output mailbox. Refused, and
    /// the frame dropped, unless this head is still that mailbox's attached
    /// head. (The journal paint-proof build publishes through
    /// `publish_journal_paint` instead.)
    #[cfg(not(feature = "journal-paint-proof"))]
    pub(crate) fn publish(&self, frame: FrameOutput) -> bool {
        self.output
            .publish_with_status(self.identity, frame, self.publication_status())
    }

    /// `publish` for the journal paint-proof fixture: the same identity check,
    /// with the paint's journal stamp.
    #[cfg(feature = "journal-paint-proof")]
    pub(crate) fn publish_journal_paint(
        &self,
        frame: FrameOutput,
        stamp: crate::JournalPaintStamp,
    ) -> bool {
        self.output
            .publish_journal_paint(self.identity, frame, stamp, self.publication_status())
    }

    /// Detach in today's order: retire the lease (its generation stops
    /// admitting), drain the output mailbox so no consumer keeps a GPU frame
    /// texture past its backend, then drop the backend.
    pub(crate) fn detach(mut self) {
        self.retire();
        let _ = self.output.retire_head(self.identity);
    }

    fn retire(&mut self) {
        #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
        fair::retire(self);
        if let Some(mut lease) = self.lease.take() {
            if let Err(error) = PROCESS.service.retire(&mut lease, Instant::now()) {
                host_log!(
                    Category::Lifecycle,
                    Level::Warn,
                    "render head retire: {error}"
                );
            }
        }
    }
}

impl Drop for Head {
    /// A head dropped without [`Head::detach`] (its slot loop ended) retires
    /// its lease and stops its mailbox accepting its frames. The last frame
    /// stays for the consumer, as before.
    fn drop(&mut self) {
        self.retire();
        self.output.release_head(self.identity);
        #[cfg(feature = "render-diagnostics")]
        emit_budget_samples(self.budget.as_ref(), self.identity, "final");
    }
}

#[cfg(feature = "render-diagnostics")]
fn emit_budget_samples(
    budget: Option<&client::render::budget::RenderBudget>,
    identity: HeadIdentity,
    stage: &'static str,
) {
    if let Some(budget) = budget {
        for sample in budget.diagnostic_snapshots() {
            sample.emit(
                stage,
                identity.slot().incarnation().get(),
                identity.generation().get(),
            );
        }
    }
}

/// Retire `head` if there is one, then drain `output`: the draw-off and
/// backend-change detach. With no head the drain still runs, so a frame a
/// previous slot loop left behind never outlives draw off.
pub(crate) fn detach(head: Option<Head>, output: Option<&FrameBuf>) {
    match head {
        Some(head) => head.detach(),
        None => {
            if let Some(output) = output {
                let _ = output.take();
            }
        }
    }
}

thread_local! {
    /// The GPU head this slot thread is painting, lent to the scene hook for
    /// one `mainredraw`.
    static GPU_FRAME: RefCell<Option<GpuFrame>> = const { RefCell::new(None) };
}

/// Lends a GPU head's lease to [`GPU_FRAME`] for one frame and takes it back
/// on drop, unwinding included.
struct LentHead<'a> {
    lease: &'a mut Option<HeadLease>,
    scenes: &'a mut u64,
}

impl<'a> LentHead<'a> {
    fn lend(lease: &'a mut Option<HeadLease>, scenes: &'a mut u64, wake: &Arc<HeadWake>) -> Self {
        let head = lease
            .take()
            .expect("an attached head holds its lease between frames");
        GPU_FRAME.with(|frame| {
            *frame.borrow_mut() = Some(GpuFrame {
                lease: head,
                wake: Arc::clone(wake),
                scenes: *scenes,
                #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
                fair: None,
            });
        });
        Self { lease, scenes }
    }
}

impl Drop for LentHead<'_> {
    fn drop(&mut self) {
        if let Some(frame) = GPU_FRAME.with(|frame| frame.borrow_mut().take()) {
            *self.lease = Some(frame.lease);
            *self.scenes = frame.scenes;
        }
    }
}

/// One GPU frame's lent head.
struct GpuFrame {
    lease: HeadLease,
    wake: Arc<HeadWake>,
    scenes: u64,
    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    fair: Option<fair::FairFrame>,
}

/// The client's scene-workspace hook. A thread painting a lent GPU head holds
/// that head's frame lease for exactly this scene submission. A thread with
/// no lent head (a renderer outside a slot head) submits directly under the
/// client's lock, as before.
fn scene_workspace(submit: &mut dyn FnMut()) {
    GPU_FRAME.with(|frame| match frame.borrow_mut().as_mut() {
        Some(frame) => {
            #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
            if frame.fair.is_some() {
                fair::submit(frame, submit);
                return;
            }
            frame.submit(&PROCESS.service, &PROCESS.counters, &mut || {
                submit();
                client::render::backend::gpu::current_scene_submit_result()
            });
        }
        None => submit(),
    });
}

impl GpuFrame {
    fn submit(
        &mut self,
        service: &RenderService,
        counters: &WorkspaceCounters,
        submit: &mut dyn FnMut() -> SceneSubmit,
    ) {
        match self.reserve(service) {
            Ok((frame, wait)) => {
                let result = submit();
                counters.leased(wait);
                match result {
                    SceneSubmit::Submitted => {
                        self.scenes += 1;
                        // Only a completed submission can replace this head's
                        // retained scene. A fault keeps its last good scene.
                        if let Err(error) =
                            frame.publish_retained_scene(SceneHandle::from_opaque(self.scenes))
                        {
                            host_log!(
                                Category::Lifecycle,
                                Level::Warn,
                                "render retained scene: {error}"
                            );
                        }
                    }
                    SceneSubmit::ResourceLimited | SceneSubmit::LockContended => {
                        let kind = if result == SceneSubmit::ResourceLimited {
                            crate::render::RenderFaultKind::ResourceLimit
                        } else {
                            crate::render::RenderFaultKind::LockContention
                        };
                        // The service counts every refusal and logs only the
                        // start of each resource-fault episode for this head.
                        if let Err(error) =
                            service.record_fault(frame.identity(), kind, Instant::now())
                        {
                            host_log!(Category::Lifecycle, Level::Warn, "render fault: {error}");
                        }
                    }
                    SceneSubmit::NotRun => host_log!(
                        Category::Lifecycle,
                        Level::Warn,
                        "render callback did not submit the consumed scene"
                    ),
                }
                if let Err(error) = frame.finish(Instant::now()) {
                    host_log!(Category::Lifecycle, Level::Warn, "render frame: {error}");
                }
            }
            Err(error) => {
                // No workspace grant (the head was retired under the frame,
                // or an identity space ran out): submit under the client's
                // lock alone, as before the wrap.
                host_log!(
                    Category::Lifecycle,
                    Level::Warn,
                    "render scene workspace: {error}; submitting under the GPU lock alone"
                );
                submit();
                counters.unleased();
            }
        }
    }

    /// Request the scene workspace and consume this head's grant, with how
    /// long it waited if the workspace was held. While another head holds
    /// the workspace, wait for this head's own grant: the wrap's stand-in for
    /// today's `GPU_SCENE_LOCK` wait. Every consume attempt also processes
    /// due expiries, so a stalled grant elsewhere expires while this head
    /// waits.
    fn reserve(
        &self,
        service: &RenderService,
    ) -> Result<(FrameLease, Option<Duration>), RenderError> {
        let mut seen = self.wake.latest();
        let requested = Instant::now();
        let (mut reservation, granted) = self.request(service)?;
        let mut waited = !granted;
        loop {
            match service.try_consume(&self.lease, reservation, Instant::now()) {
                Ok(frame) => return Ok((frame, waited.then(|| requested.elapsed()))),
                // Still queued behind the head that holds the workspace.
                Err(RenderError::NotGranted) => {
                    seen = self.wake.wait_past(seen, GRANT_WAIT_SLICE);
                }
                // Stalled past the grant lifetime, or the request no longer
                // stands: ask again.
                Err(
                    RenderError::Expired | RenderError::Cancelled | RenderError::StaleReservation,
                ) => {
                    (reservation, _) = self.request(service)?;
                    waited = true;
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn request(&self, service: &RenderService) -> Result<(ReservationIdentity, bool), RenderError> {
        let now = Instant::now();
        Ok(
            match service.request_frame(&self.lease, Deadline::at(now), now)? {
                RequestOutcome::Granted(identity) => (identity, true),
                RequestOutcome::Busy(identity) => (identity, false),
            },
        )
    }
}

/// A head's notification sink: the latest publication sequence the service
/// announced for it, and the condition a waiting GPU head sleeps on.
#[cfg_attr(
    not(any(feature = "render-diagnostics", feature = "render-test-hooks")),
    derive(Default)
)]
struct HeadWake {
    latest: Mutex<PublishSequence>,
    published: Condvar,
    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    owner: std::thread::ThreadId,
    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    slot_wake: Arc<dyn Notifier>,
    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    latest_cross_notice: Mutex<Option<(PublishSequence, Instant)>>,
}

#[cfg(all(
    test,
    any(feature = "render-diagnostics", feature = "render-test-hooks")
))]
impl Default for HeadWake {
    fn default() -> Self {
        let (wake, _park) = crate::wake_channel();
        Self::new(Arc::new(wake))
    }
}

impl HeadWake {
    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    fn new(slot_wake: Arc<dyn Notifier>) -> Self {
        Self {
            latest: Mutex::new(PublishSequence::default()),
            published: Condvar::new(),
            owner: std::thread::current().id(),
            slot_wake,
            latest_cross_notice: Mutex::new(None),
        }
    }

    fn latest(&self) -> PublishSequence {
        *lock(&self.latest)
    }

    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    fn grant_notice(&self, sequence: PublishSequence) -> Option<Instant> {
        lock(&self.latest_cross_notice)
            .filter(|(published, _)| *published == sequence)
            .map(|(_, notified_at)| notified_at)
    }

    /// Sleep until a publication after `seen`, for at most `slice`. Returns
    /// the latest sequence.
    fn wait_past(&self, seen: PublishSequence, slice: Duration) -> PublishSequence {
        let latest = lock(&self.latest);
        let (latest, _) = self
            .published
            .wait_timeout_while(latest, slice, |latest| *latest <= seen)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *latest
    }
}

impl Notifier for HeadWake {
    fn notify(&self, notice: Notice) {
        let mut latest = lock(&self.latest);
        *latest = (*latest).max(notice.sequence());
        drop(latest);
        self.published.notify_all();
        #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
        if fair::enabled() && std::thread::current().id() != self.owner {
            *lock(&self.latest_cross_notice) = Some((notice.sequence(), Instant::now()));
            self.slot_wake.notify(notice);
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "render-diagnostics")]
    #[test]
    fn budget_observation_clock_never_repeats_or_advances_before_its_boundary() {
        let start = Instant::now();
        let mut clock = super::BudgetObservationClock(start);
        assert!(!clock.due(start - std::time::Duration::from_secs(1)));
        assert!(!clock.due(start + std::time::Duration::from_millis(999)));
        assert!(clock.due(start + std::time::Duration::from_secs(1)));
        assert!(!clock.due(start + std::time::Duration::from_secs(1)));
        assert!(!clock.due(start + std::time::Duration::from_millis(1999)));
        assert!(clock.due(start + std::time::Duration::from_secs(2)));
    }

    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::atomic::AtomicBool;
    use std::thread;

    use super::*;
    use crate::render::ReservationState;

    #[cfg(not(feature = "render-diagnostics"))]
    #[test]
    fn non_diagnostic_head_preserves_its_baseline_owner_layout() {
        #[allow(dead_code)]
        struct BaselineHead {
            lease: Option<HeadLease>,
            identity: HeadIdentity,
            wake: Arc<HeadWake>,
            output: Arc<FrameBuf>,
            scenes: u64,
            renderer: Renderer,
            prefer_cpu: bool,
            lowmem: bool,
            #[cfg(feature = "render-test-hooks")]
            fair: fair::FairHead,
            #[cfg(feature = "render-test-hooks")]
            budget: Option<client::render::budget::RenderBudget>,
        }
        assert_eq!(
            std::mem::size_of::<Head>(),
            std::mem::size_of::<BaselineHead>()
        );
        assert_eq!(
            std::mem::align_of::<Head>(),
            std::mem::align_of::<BaselineHead>()
        );
    }

    /// The F0 fixture lifetime: short enough for a stalled grant to expire
    /// inside a test.
    const TTL: Duration = Duration::from_millis(20);

    fn service(ttl: Duration) -> (RenderService, RenderPumpLease) {
        let service = RenderService::new(ttl).unwrap();
        let pump = service.register_pump().unwrap();
        (service, pump)
    }

    fn slot(raw: u64) -> SlotKey {
        SlotKey::from_incarnation(raw).unwrap()
    }

    /// A GPU head as the scene hook sees it, on a test-local service.
    fn gpu_frame(service: &RenderService, pump: &RenderPumpLease, raw: u64) -> GpuFrame {
        let wake = Arc::new(HeadWake::default());
        let notifier: Arc<dyn Notifier> = wake.clone();
        let lease = service.attach(pump, slot(raw), notifier).unwrap();
        GpuFrame {
            lease,
            wake,
            scenes: 0,
            #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
            fair: None,
        }
    }

    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn unset_production_quantum_keeps_dev_wrap_ttl_and_no_heartbeat() {
        let process = build_process_render();
        #[cfg(feature = "render-test-hooks")]
        assert!(
            !fair::enabled(),
            "this unit-test binary does not configure the process"
        );
        assert_eq!(WRAP_GRANT_TTL, Duration::from_millis(100));
        let head = gpu_frame(&process.service, &process.pump, 29);
        let now = Instant::now();
        let RequestOutcome::Granted(reservation) = process
            .service
            .request_frame(&head.lease, Deadline::at(now), now)
            .unwrap()
        else {
            panic!("an idle wrap grants its first request");
        };
        assert_eq!(
            head.lease.snapshot().1.expires_at(),
            Some(Deadline::at(now + WRAP_GRANT_TTL))
        );
        let lease = process
            .service
            .try_consume(&head.lease, reservation, now)
            .unwrap();
        assert_eq!(
            process
                .service
                .pump(&process.pump, now + Duration::from_secs(1))
                .unwrap(),
            None
        );
        assert_eq!(process.service.fault_stats().totals(), [0, 0, 0]);
        assert!(process.service.fault_stats().latest().is_none());
        drop(lease);
    }

    #[test]
    fn unset_production_quantum_never_publishes_recovered_fault_metadata() {
        use client::client::{ClientConfig, ClientRevision};
        use client::graphics::PixMap;
        use client::render::backend::RenderFault;
        use client::render::budget::{RenderLimit, RenderResource};

        #[cfg(feature = "render-test-hooks")]
        assert!(!fair::enabled(), "this test does not configure the process");
        let output = FrameBuf::new();
        let wake: Arc<dyn Notifier> = Arc::new(HeadWake::default());
        let render = SlotRender::new(slot(930), Arc::clone(&output), wake);
        let mut head = Head::attach(&render, true, false).unwrap();
        let mut client = Client::new_with_revision(
            ClientConfig {
                host: "127.0.0.1".into(),
                port: 43594,
                cache_dir: "/nonexistent-unset-render-fault-fixture".into(),
                members: true,
                lowmem: false,
            },
            ClientRevision::R289,
        );
        client.ingame = true;
        client.draw = true;
        let fault = RenderLimit::new(RenderResource::NavInput, 2, 1);
        assert_eq!(client.set_budgeted_nav_debug_paint(Err(fault)), Err(fault));
        // Inject a receipt without enabling the bounded host path. The
        // production publication filter must not forward this new metadata.
        let refused = head.paint(&mut client);
        assert!(matches!(refused, FrameOutput::Refused(_)));
        assert_eq!(
            head.renderer.render_fault(),
            Some(RenderFault::Budget(fault))
        );
        let usable = FrameOutput::PixMap(PixMap::new(1, 1));
        #[cfg(not(feature = "journal-paint-proof"))]
        assert!(head.publish(usable));
        #[cfg(feature = "journal-paint-proof")]
        assert!(head.publish_journal_paint(
            usable,
            crate::JournalPaintStamp {
                modal_root: -1,
                journal_paint_hidden: false,
            },
        ));
        assert_eq!(output.generation(), 1);
        assert_eq!(output.render_fault(), None);
        head.detach();
    }

    #[cfg(feature = "render-test-hooks")]
    #[test]
    fn configured_cpu_head_publishes_usable_recovery_and_fault_metadata() {
        use client::client::{ClientConfig, ClientPlayer, ClientRevision};
        use client::render::backend::RenderFault;
        use client::render::budget::{RenderLimit, RenderResource};
        use fair::test_hooks::{configure_process, CaptureFixture, ProcessFixture};

        const CHILD: &str = "T44_CPU_RECOVERY_METADATA_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "render_head::tests::configured_cpu_head_publishes_usable_recovery_and_fault_metadata",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .output()
                .expect("spawn an isolated configured CPU head");
            assert!(
                output.status.success(),
                "configured CPU child failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            return;
        }
        configure_process(ProcessFixture {
            grant_ttl: Duration::from_millis(20),
            quantum: Duration::from_millis(40),
        })
        .unwrap();
        let mut fixture = CaptureFixture::new(slot(931), true, false).unwrap();
        let mut client = Client::new_with_revision(
            ClientConfig {
                host: "127.0.0.1".into(),
                port: 43594,
                cache_dir: "/nonexistent-configured-cpu-recovery-fixture".into(),
                members: true,
                lowmem: false,
            },
            ClientRevision::R289,
        );
        client.ingame = true;
        client.draw = true;
        client.scene_state = 2;
        client.local_player = Some(ClientPlayer::at(50, 50));
        client.world.fill_base_level(0);
        assert!(fixture.paint(&mut client));
        let baseline = fixture.output().take_presented().unwrap();
        assert!(baseline.stamp().has_valid_scene());
        assert_eq!(fixture.output().render_fault(), None);
        let baseline_sequence = baseline.stamp().mailbox_sequence();
        drop(baseline);

        let fault = RenderLimit::new(RenderResource::NavInput, 2, 1);
        assert_eq!(client.set_budgeted_nav_debug_paint(Err(fault)), Err(fault));
        assert!(fixture.paint(&mut client));
        assert_eq!(
            fixture.output().render_fault(),
            Some(RenderFault::Budget(fault))
        );
        let recovered = fixture.output().take_presented().unwrap();
        assert!(recovered.stamp().has_valid_scene());
        assert_eq!(recovered.stamp().mailbox_sequence(), baseline_sequence + 1);
        assert!(matches!(recovered.output(), FrameOutput::PixMap(_)));

        client.set_nav_debug_paint(None);
        assert!(fixture.paint(&mut client));
        assert_eq!(fixture.output().render_fault(), None);
    }

    #[test]
    fn default_vertex_cap_fault_logs_once_and_keeps_the_retained_scene() {
        const CHILD: &str = "T44_DEFAULT_CAP_LOG_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "render_head::tests::default_vertex_cap_fault_logs_once_and_keeps_the_retained_scene",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .output()
                .expect("spawn an isolated default render log test");
            assert!(
                output.status.success(),
                "default render log child failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            return;
        }
        struct Capture(Mutex<Vec<(Level, String)>>);
        impl api::hostlog::Sink for Capture {
            fn record(&self, record: &api::hostlog::Record<'_>) {
                lock(&self.0).push((record.level, record.message.to_owned()));
            }
        }
        static CAPTURE: Capture = Capture(Mutex::new(Vec::new()));
        assert!(api::hostlog::install_sink(&CAPTURE));
        #[cfg(feature = "render-test-hooks")]
        assert!(!fair::enabled(), "the fair gate must remain unset");
        let (service, pump) = service(WRAP_GRANT_TTL);
        let counters = WorkspaceCounters::default();
        let mut frame = gpu_frame(&service, &pump, 932);
        frame.submit(&service, &counters, &mut || SceneSubmit::Submitted);
        let retained = frame.lease.snapshot().1.retained_scene();
        assert_eq!(retained, Some(SceneHandle::from_opaque(1)));
        // The client's checked vertex-cap failure returns ResourceLimited
        // from this same callback, without submitting a replacement image.
        for _ in 0..3 {
            frame.submit(&service, &counters, &mut || SceneSubmit::ResourceLimited);
            assert_eq!(frame.lease.snapshot().1.retained_scene(), retained);
            assert_eq!(frame.lease.snapshot().1.state(), ReservationState::Idle);
        }
        assert_eq!(frame.scenes, 1);
        assert_eq!(service.fault_stats().totals(), [0, 0, 3]);
        assert_eq!(
            lock(&CAPTURE.0).len(),
            1,
            "one warning while the fault continues"
        );

        // Each head owns its episode. Another head's recovery cannot reset it.
        let mut peer = gpu_frame(&service, &pump, 933);
        peer.submit(&service, &counters, &mut || SceneSubmit::ResourceLimited);
        assert_eq!(lock(&CAPTURE.0).len(), 2);
        peer.submit(&service, &counters, &mut || SceneSubmit::Submitted);
        frame.submit(&service, &counters, &mut || SceneSubmit::ResourceLimited);
        assert_eq!(lock(&CAPTURE.0).len(), 2);

        frame.submit(&service, &counters, &mut || SceneSubmit::Submitted);
        let recovered = frame.lease.snapshot().1.retained_scene();
        assert_eq!(recovered, Some(SceneHandle::from_opaque(2)));
        for _ in 0..2 {
            frame.submit(&service, &counters, &mut || SceneSubmit::ResourceLimited);
            assert_eq!(frame.lease.snapshot().1.retained_scene(), recovered);
        }
        assert_eq!(frame.lease.snapshot().1.state(), ReservationState::Idle);
        assert_eq!(service.fault_stats().totals(), [0, 0, 7]);
        let lines = lock(&CAPTURE.0);
        assert_eq!(lines.len(), 3, "one warning per head and fault episode");
        assert!(lines.iter().all(|line| line.0 == Level::Warn));
        assert!(lines[0].1.starts_with("render fault #1 ResourceLimit "));
        assert!(lines[1].1.starts_with("render fault #4 ResourceLimit "));
        assert!(lines[2].1.starts_with("render fault #6 ResourceLimit "));
    }

    #[test]
    fn an_idle_workspace_leases_the_scene_and_retains_it_per_head() {
        let (service, pump) = service(TTL);
        let counters = WorkspaceCounters::default();
        let mut first = gpu_frame(&service, &pump, 1);
        let second = gpu_frame(&service, &pump, 2);
        let mut ran = 0;
        first.submit(&service, &counters, &mut || {
            ran += 1;
            SceneSubmit::Submitted
        });
        first.submit(&service, &counters, &mut || {
            ran += 1;
            SceneSubmit::Submitted
        });
        assert_eq!(ran, 2);
        let (sequence, snapshot) = first.lease.snapshot();
        assert_eq!(snapshot.state(), ReservationState::Idle);
        assert_eq!(snapshot.retained_scene(), Some(SceneHandle::from_opaque(2)));
        assert_eq!(
            sequence.get(),
            10,
            "each scene publishes Pending, Granted, Consumed, its retained scene and Idle"
        );
        assert_eq!(
            second.lease.snapshot().1.retained_scene(),
            None,
            "a retained scene is per head"
        );
        let stats = counters.stats();
        assert_eq!(
            (stats.leased(), stats.waited(), stats.unleased()),
            (2, 0, 0)
        );
    }

    #[test]
    fn a_busy_head_waits_for_its_own_grant_then_submits() {
        let (service, pump) = service(WRAP_GRANT_TTL);
        let counters = WorkspaceCounters::default();
        let holder = service
            .attach(&pump, slot(3), Arc::new(HeadWake::default()))
            .unwrap();
        let now = Instant::now();
        let RequestOutcome::Granted(grant) = service
            .request_frame(&holder, Deadline::at(now), now)
            .unwrap()
        else {
            panic!("an idle workspace grants at once");
        };
        let held = service.try_consume(&holder, grant, now).unwrap();

        let mut waiter = gpu_frame(&service, &pump, 4);
        let wake = Arc::clone(&waiter.wake);
        let ran = AtomicBool::new(false);
        thread::scope(|scope| {
            let submitting = scope.spawn(|| {
                waiter.submit(&service, &counters, &mut || {
                    ran.store(true, Ordering::SeqCst);
                    SceneSubmit::Submitted
                });
            });
            wait_until("the waiter's Pending publication", || {
                wake.latest().get() >= 1
            });
            thread::sleep(Duration::from_millis(30));
            assert!(
                !ran.load(Ordering::SeqCst),
                "no scene submits while another head holds the workspace"
            );
            held.finish(Instant::now()).unwrap();
            submitting.join().unwrap();
        });
        assert!(ran.load(Ordering::SeqCst));
        let snapshot = waiter.lease.snapshot().1;
        assert_eq!(snapshot.state(), ReservationState::Idle);
        assert_eq!(snapshot.retained_scene(), Some(SceneHandle::from_opaque(1)));
        let stats = counters.stats();
        assert_eq!((stats.leased(), stats.waited()), (1, 1));
        assert!(stats.wait() >= Duration::from_millis(30));
        assert_eq!(stats.max_wait(), stats.wait());
    }

    /// §2.7: a head that stops after a grant cannot hold the workspace. The
    /// waiting head polls the service between waits, so the stalled grant
    /// expires at its deadline and the waiter proceeds; its queued request
    /// borrows nothing.
    #[test]
    fn a_stalled_grant_expires_and_the_waiting_head_proceeds() {
        let (service, pump) = service(TTL);
        let counters = WorkspaceCounters::default();
        let stalled = service
            .attach(&pump, slot(5), Arc::new(HeadWake::default()))
            .unwrap();
        let granted_at = Instant::now();
        assert!(matches!(
            service
                .request_frame(&stalled, Deadline::at(granted_at), granted_at)
                .unwrap(),
            RequestOutcome::Granted(_)
        ));

        let mut waiter = gpu_frame(&service, &pump, 6);
        let mut ran = 0;
        waiter.submit(&service, &counters, &mut || {
            ran += 1;
            SceneSubmit::Submitted
        });
        assert_eq!(ran, 1);
        assert!(
            granted_at.elapsed() >= TTL,
            "the waiter proceeds only after the stalled grant's expiry"
        );
        assert_eq!(stalled.snapshot().1.state(), ReservationState::Expired);
        assert_eq!(
            waiter.lease.snapshot().1.retained_scene(),
            Some(SceneHandle::from_opaque(1))
        );
        assert_eq!(counters.stats().waited(), 1);
    }

    #[test]
    fn a_retired_head_submits_under_the_lock_alone_and_publishes_nothing() {
        let (service, pump) = service(TTL);
        let counters = WorkspaceCounters::default();
        let mut frame = gpu_frame(&service, &pump, 7);
        service.retire(&mut frame.lease, Instant::now()).unwrap();
        let mut ran = 0;
        frame.submit(&service, &counters, &mut || {
            ran += 1;
            SceneSubmit::Submitted
        });
        assert_eq!(ran, 1, "the scene still submits, as before the wrap");
        assert_eq!(frame.lease.snapshot().1.retained_scene(), None);
        let stats = counters.stats();
        assert_eq!((stats.leased(), stats.unleased()), (0, 1));
    }

    #[test]
    fn the_scene_hook_without_a_lent_head_submits_directly() {
        let mut ran = 0;
        scene_workspace(&mut || ran += 1);
        assert_eq!(ran, 1);
    }

    #[test]
    fn a_lent_head_comes_back_after_a_panicking_frame() {
        let (service, pump) = service(TTL);
        let wake = Arc::new(HeadWake::default());
        let notifier: Arc<dyn Notifier> = wake.clone();
        let mut lease = Some(service.attach(&pump, slot(8), notifier).unwrap());
        let mut scenes = 3;
        let frame = catch_unwind(AssertUnwindSafe(|| {
            let _lent = LentHead::lend(&mut lease, &mut scenes, &wake);
            assert!(GPU_FRAME.with(|frame| frame.borrow().is_some()));
            panic!("a frame that panics mid-paint");
        }));
        assert!(frame.is_err());
        assert!(lease.is_some(), "the lease returns to its head");
        assert_eq!(scenes, 3);
        assert!(GPU_FRAME.with(|frame| frame.borrow().is_none()));
    }

    /// Not a gate: the wrap's grant wait against a bare mutex over the same
    /// busy section, at 1, 8 and 16 contending threads. Run in release:
    /// `cargo test -p host --release --lib grant_wait_against_bare_mutex --
    /// --ignored --nocapture`.
    #[test]
    #[ignore = "benchmark for the T4.2 report"]
    fn grant_wait_against_bare_mutex() {
        fn spin(span: Duration) {
            let start = Instant::now();
            while start.elapsed() < span {
                std::hint::spin_loop();
            }
        }
        for section in [Duration::from_micros(200), Duration::from_millis(2)] {
            for threads in [1u64, 8, 16] {
                let rounds = (Duration::from_secs(2).as_nanos()
                    / (section.as_nanos() * u128::from(threads)))
                    as u64;
                let mutex = Mutex::new(());
                let start = Instant::now();
                thread::scope(|scope| {
                    for _ in 0..threads {
                        scope.spawn(|| {
                            for _ in 0..rounds {
                                let _section = lock(&mutex);
                                spin(section);
                            }
                        });
                    }
                });
                let bare = start.elapsed();

                let (service, pump) = service(WRAP_GRANT_TTL);
                let counters = WorkspaceCounters::default();
                let (service, counters) = (&service, &counters);
                let start = Instant::now();
                thread::scope(|scope| {
                    for thread in 0..threads {
                        let mut frame = gpu_frame(service, &pump, 100 + thread);
                        scope.spawn(move || {
                            for _ in 0..rounds {
                                frame.submit(service, counters, &mut || {
                                    spin(section);
                                    SceneSubmit::Submitted
                                });
                            }
                        });
                    }
                });
                let leased = start.elapsed();
                let stats = counters.stats();
                let sections = rounds * threads;
                let per_section = |elapsed: Duration| elapsed.as_nanos() / u128::from(sections);
                println!(
                    "GRANTBENCH section_us={} threads={threads} sections={sections} bare_ns_per_section={} leased_ns_per_section={} overhead_ns_per_section={} waited={} mean_wait_us={} max_wait_us={}",
                    section.as_micros(),
                    per_section(bare),
                    per_section(leased),
                    per_section(leased) as i128 - per_section(bare) as i128,
                    stats.waited(),
                    stats.wait().as_micros() / u128::from(stats.waited().max(1)),
                    stats.max_wait().as_micros(),
                );
            }
        }
    }
}
