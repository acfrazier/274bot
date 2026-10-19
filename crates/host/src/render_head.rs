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
use client::render::backend::{BackendKind, FrameOutput};
use client::render::Renderer;

use crate::render::{
    FrameLease, HeadIdentity, HeadLease, RenderError, RenderPumpLease, RenderService,
    RequestOutcome, ReservationIdentity, SceneHandle,
};
use crate::FrameBuf;

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

static PROCESS: LazyLock<ProcessRender> = LazyLock::new(|| {
    client::render::backend::gpu::set_scene_workspace_hook(scene_workspace);
    let service =
        RenderService::new(WRAP_GRANT_TTL).expect("the wrap grant lifetime is positive and finite");
    let pump = service
        .register_pump()
        .expect("a new service registers its one pump");
    ProcessRender {
        service,
        pump,
        counters: WorkspaceCounters::default(),
    }
});

/// Run the process render pump at `now` and return its next deadline. The
/// headed composition calls it from its own loop (the panel window), so a
/// grant whose head stopped consuming expires at its deadline even when no
/// slot asks for a frame.
pub fn pump(now: Instant) -> Option<Deadline> {
    match PROCESS.service.pump(&PROCESS.pump, now) {
        Ok(deadline) => deadline,
        Err(error) => {
            host_log!(Category::Lifecycle, Level::Warn, "render pump: {error}");
            None
        }
    }
}

/// What GPU scene submissions in this process have cost in workspace
/// admission so far: the evidence for the wrap's retained wait.
pub fn workspace_stats() -> WorkspaceStats {
    PROCESS.counters.stats()
}

/// A snapshot of the process's GPU scene-workspace counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkspaceStats {
    leased: u64,
    waited: u64,
    wait_ns: u64,
    max_wait_ns: u64,
    unleased: u64,
}

impl WorkspaceStats {
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
}

impl SlotRender {
    /// The envelope for one slot worker.
    pub fn new(slot: SlotKey, output: Arc<FrameBuf>) -> Self {
        Self { slot, output }
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
        let wake = Arc::new(HeadWake::default());
        let notifier: Arc<dyn Notifier> = wake.clone();
        let lease = PROCESS
            .service
            .attach(&PROCESS.pump, render.slot, notifier)?;
        let identity = lease.identity();
        // GPU-first (the host default): the head prefers the wgpu backend,
        // with `CpuBackend` as the fallback on wgpu init failure
        // (`Renderer::new_prefer` selects, `Renderer::backend_kind` reports);
        // `BOT_CPU=1` forces the CPU fidelity path. GPU set-up blocks on the
        // driver: not time the host kept us waiting.
        let renderer = crate::capacity::off_cpu_wait(|| Renderer::new_prefer(lowmem, !prefer_cpu));
        render.output.attach_head(identity);
        Ok(Self {
            lease: Some(lease),
            identity,
            wake,
            output: Arc::clone(&render.output),
            scenes: 0,
            renderer,
            prefer_cpu,
            lowmem,
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

    /// Paint one frame. A GPU head lends its lease to this thread's scene
    /// hook for the frame, so its scene submission runs under a frame lease.
    pub(crate) fn paint(&mut self, client: &mut Client) -> FrameOutput {
        if self.renderer.backend_kind() != BackendKind::Gpu {
            return self.renderer.mainredraw(client);
        }
        let _lent = LentHead::lend(&mut self.lease, &mut self.scenes, &self.wake);
        self.renderer.mainredraw(client)
    }

    /// Publish a painted frame into the slot's output mailbox. Refused, and
    /// the frame dropped, unless this head is still that mailbox's attached
    /// head. (The journal paint-proof build publishes through
    /// `publish_journal_paint` instead.)
    #[cfg(not(feature = "journal-paint-proof"))]
    pub(crate) fn publish(&self, frame: FrameOutput) -> bool {
        self.output.publish(self.identity, frame)
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
            .publish_journal_paint(self.identity, frame, stamp)
    }

    /// Detach in today's order: retire the lease (its generation stops
    /// admitting), drain the output mailbox so no consumer keeps a GPU frame
    /// texture past its backend, then drop the backend.
    pub(crate) fn detach(mut self) {
        self.retire();
        let _ = self.output.retire_head(self.identity);
    }

    fn retire(&mut self) {
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
}

/// The client's scene-workspace hook. A thread painting a lent GPU head holds
/// that head's frame lease for exactly this scene submission. A thread with
/// no lent head (a renderer outside a slot head) submits directly under the
/// client's lock, as before.
fn scene_workspace(submit: &mut dyn FnMut()) {
    GPU_FRAME.with(|frame| match frame.borrow_mut().as_mut() {
        Some(frame) => frame.submit(&PROCESS.service, &PROCESS.counters, submit),
        None => submit(),
    });
}

impl GpuFrame {
    fn submit(
        &mut self,
        service: &RenderService,
        counters: &WorkspaceCounters,
        submit: &mut dyn FnMut(),
    ) {
        match self.reserve(service) {
            Ok((frame, wait)) => {
                submit();
                counters.leased(wait);
                self.scenes += 1;
                // Generation-checked: refused once this head is retired. The
                // handle names this head's scene as of its n-th submission.
                if let Err(error) =
                    frame.publish_retained_scene(SceneHandle::from_opaque(self.scenes))
                {
                    host_log!(
                        Category::Lifecycle,
                        Level::Warn,
                        "render retained scene: {error}"
                    );
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
#[derive(Default)]
struct HeadWake {
    latest: Mutex<PublishSequence>,
    published: Condvar,
}

impl HeadWake {
    fn latest(&self) -> PublishSequence {
        *lock(&self.latest)
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
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::atomic::AtomicBool;
    use std::thread;

    use super::*;
    use crate::render::ReservationState;

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
    fn an_idle_workspace_leases_the_scene_and_retains_it_per_head() {
        let (service, pump) = service(TTL);
        let counters = WorkspaceCounters::default();
        let mut first = gpu_frame(&service, &pump, 1);
        let second = gpu_frame(&service, &pump, 2);
        let mut ran = 0;
        first.submit(&service, &counters, &mut || ran += 1);
        first.submit(&service, &counters, &mut || ran += 1);
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
                    ran.store(true, Ordering::SeqCst)
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
        waiter.submit(&service, &counters, &mut || ran += 1);
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
        frame.submit(&service, &counters, &mut || ran += 1);
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
                                frame.submit(service, counters, &mut || spin(section));
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
