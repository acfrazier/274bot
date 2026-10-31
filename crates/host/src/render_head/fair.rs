//! Opt-in fair-render calibration and test hooks. Production keeps the wrap by default.
use std::cell::Cell;
#[cfg(feature = "render-test-hooks")]
use std::cell::RefCell;
#[cfg(feature = "render-test-hooks")]
use std::marker::PhantomData;
use std::num::NonZeroU64;
#[cfg(feature = "render-test-hooks")]
use std::rc::Rc;
use std::sync::OnceLock;

use client::render::backend::gpu::{SceneAdmission, SceneSubmit};
use client::render::backend::SceneProgress;

use super::*;
use crate::render::ReservationState;

#[cfg(feature = "render-test-hooks")]
pub mod test_hooks {
    use super::*;

    /// Explicit integration-test configuration, never a production default.
    #[derive(Clone, Copy)]
    pub struct ProcessFixture {
        pub grant_ttl: Duration,
        pub quantum: Duration,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum RenderTestPoint {
        AfterConsume,
        BeforeTerminal,
        BeforeLeaseRelease,
        AfterLeaseRelease,
        OverlayEntry,
    }

    #[derive(Debug, Clone, Copy)]
    pub struct RenderTestEvent {
        pub head: HeadIdentity,
        pub point: RenderTestPoint,
        pub owns_lease: bool,
        pub reservation: Option<ReservationIdentity>,
    }

    /// Paint-only crosshair input, restored before the next simulation step.
    #[derive(Debug, Clone, Copy)]
    pub enum RenderTestCross {
        Hidden,
        At { x: i32, y: i32 },
    }

    pub trait RenderTestControl: Send + Sync {
        fn now(&self) -> Instant;
        fn loading_paint(&self) -> bool {
            false
        }
        fn cross_paint(&self) -> Option<RenderTestCross> {
            None
        }
        fn resource_failure(
            &self,
        ) -> Option<client::render::backend::gpu::test_hooks::SceneTestPoint> {
            None
        }
        /// Observe a completed simulation pass, independent of paint cadence.
        fn client_cycle(
            &self,
            _slot: SlotKey,
            _backend: BackendKind,
            _cycle: i32,
            _complete_game_scene: bool,
        ) {
        }
        fn at(&self, event: RenderTestEvent);
    }

    pub fn configure_process(fixture: ProcessFixture) -> Result<(), RenderError> {
        let mut config = lock(&CONFIG);
        if config.started || config.fixture.is_some() || CALIBRATION.get().is_some() {
            return Err(RenderError::ConfigurationLocked);
        }
        // Validate before storing the fixture, without initializing PROCESS.
        let calibration = CalibrationConfig {
            grant_ttl: fixture.grant_ttl,
            scene_quantum: fixture.quantum,
        };
        validate_calibration(calibration)?;
        CALIBRATION
            .set(calibration)
            .map_err(|_| RenderError::ConfigurationLocked)?;
        config.fixture = Some(fixture);
        Ok(())
    }

    /// Supply fixture-owned memory ceilings after fair admission is configured,
    /// and before the first process service or head is constructed.
    pub fn configure_render_budgets(
        limits: client::render::budget::RenderBudgetLimits,
    ) -> Result<(), RenderError> {
        let mut config = lock(&CONFIG);
        if config.started || config.fixture.is_none() || config.budget.is_some() {
            return Err(RenderError::ConfigurationLocked);
        }
        config.budget = Some(client::render::budget::RenderBudget::new(limits));
        Ok(())
    }

    /// Charge headed shared-device initialization to the configured process
    /// budget before any slot constructs its own head-local budget scope.
    pub fn with_process_render_budget<T>(action: impl FnOnce() -> T) -> T {
        super::with_process_render_budget(action)
    }

    /// Install a caller-owned control for real slots created by a headed
    /// integration fixture. This cannot enable fair admission on its own.
    #[cfg(feature = "render-test-hooks")]
    pub fn configure_process_control(
        control: Arc<dyn RenderTestControl>,
    ) -> Result<(), RenderError> {
        let mut config = lock(&CONFIG);
        if config.started || config.fixture.is_none() || config.control.is_some() {
            return Err(RenderError::ConfigurationLocked);
        }
        config.control = Some(control);
        Ok(())
    }

    /// Genuine producer used by cross-crate capture tests. Stamps can only
    /// leave this fixture through the real renderer and mailbox publisher.
    pub struct CaptureFixture {
        head: Head,
        render: SlotRender,
        _park: crate::SlotPark,
    }

    impl CaptureFixture {
        pub fn new(slot: SlotKey, prefer_cpu: bool, lowmem: bool) -> Result<Self, RenderError> {
            let output = FrameBuf::new();
            let (wake, park) = crate::wake_channel();
            let render = SlotRender::new(slot, output, Arc::new(wake));
            let head = Head::attach(&render, prefer_cpu, lowmem)?;
            Ok(Self {
                head,
                render,
                _park: park,
            })
        }

        pub fn output(&self) -> &Arc<FrameBuf> {
            self.render.output()
        }

        pub fn backend_kind(&self) -> BackendKind {
            self.head.backend_kind()
        }

        pub fn note_due(&mut self, now: Instant) -> Result<(), RenderError> {
            self.head.note_due(now)
        }

        pub fn paint(&mut self, client: &mut Client) -> bool {
            let frame = self.head.paint(client);
            #[cfg(not(feature = "journal-paint-proof"))]
            {
                self.head.publish(frame)
            }
            #[cfg(feature = "journal-paint-proof")]
            {
                self.head.publish_journal_paint(
                    frame,
                    crate::JournalPaintStamp {
                        modal_root: client.main_modal_id,
                        journal_paint_hidden: client.journal_paint_hidden(),
                    },
                )
            }
        }
    }

    /// Fixed render scalar storage grouped by the accepted memory rows.
    ///
    /// `s_head` includes the inline `Head` state other than `Renderer` (already
    /// reported by the renderer census), the per-head HeadWake and HeadCell Arc
    /// allocation blocks, enabled head-local budget states, and head-thread
    /// TLS. It counts neither the aliased HeadWake notifier payload nor the
    /// pre-existing SlotWake payload. The caller-supplied RenderTestControl Arc
    /// payload is fixture-owned and is not represented as zero-byte storage.
    /// The mailbox row is the no-hooks production layout, not the larger instrumented `FrameBuf`.
    /// The no-hooks slot-I/O receipt measures that layout against the actual production type.
    /// `s_consumer_active_bytes` omits the gate block already present in `s_gate`; `s_consumer`
    /// is the draining receipt, which adds that gate block once when its producer has retired.
    #[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
    pub struct ScalarReceipt {
        s_head: usize,
        s_gate: usize,
        s_consumer_active_bytes: usize,
        s_consumer: usize,
        s_service: usize,
        mailbox_per_loaded_bot_bytes: usize,
        head_inline_bytes: usize,
        head_thread_local_bytes: usize,
        head_wake_arc_allocation_bytes: usize,
        head_cell_arc_allocation_bytes: usize,
        head_budget_state_allocation_count: usize,
        head_budget_arc_allocation_bytes: usize,
        render_process_static_bytes: usize,
        fair_process_static_bytes: usize,
        render_service_arc_allocation_bytes: usize,
        process_budget_state_allocation_count: usize,
        process_budget_arc_allocation_bytes: usize,
        present_gate_arc_allocation_bytes: usize,
        present_retained_arc_allocation_bytes: usize,
        present_registration_descriptor_bytes: usize,
        present_held_frame_descriptor_bytes: usize,
        present_draining_gate_arc_allocation_bytes: usize,
        present_process_service_bytes: usize,
    }

    impl ScalarReceipt {
        pub const fn s_head(self) -> usize {
            self.s_head
        }
        pub const fn s_gate(self) -> usize {
            self.s_gate
        }
        pub const fn s_consumer_active_bytes(self) -> usize {
            self.s_consumer_active_bytes
        }
        pub const fn s_consumer(self) -> usize {
            self.s_consumer
        }
        pub const fn s_service(self) -> usize {
            self.s_service
        }
        pub const fn mailbox_per_loaded_bot_bytes(self) -> usize {
            self.mailbox_per_loaded_bot_bytes
        }
        pub const fn head_inline_bytes(self) -> usize {
            self.head_inline_bytes
        }
        pub const fn head_thread_local_bytes(self) -> usize {
            self.head_thread_local_bytes
        }
        pub const fn head_wake_arc_allocation_bytes(self) -> usize {
            self.head_wake_arc_allocation_bytes
        }
        pub const fn head_cell_arc_allocation_bytes(self) -> usize {
            self.head_cell_arc_allocation_bytes
        }
        pub const fn head_budget_state_allocation_count(self) -> usize {
            self.head_budget_state_allocation_count
        }
        pub const fn head_budget_arc_allocation_bytes(self) -> usize {
            self.head_budget_arc_allocation_bytes
        }
        pub const fn render_process_static_bytes(self) -> usize {
            self.render_process_static_bytes
        }
        pub const fn fair_process_static_bytes(self) -> usize {
            self.fair_process_static_bytes
        }
        pub const fn render_service_arc_allocation_bytes(self) -> usize {
            self.render_service_arc_allocation_bytes
        }
        pub const fn process_budget_state_allocation_count(self) -> usize {
            self.process_budget_state_allocation_count
        }
        pub const fn process_budget_arc_allocation_bytes(self) -> usize {
            self.process_budget_arc_allocation_bytes
        }
        pub const fn present_gate_arc_allocation_bytes(self) -> usize {
            self.present_gate_arc_allocation_bytes
        }
        pub const fn present_retained_arc_allocation_bytes(self) -> usize {
            self.present_retained_arc_allocation_bytes
        }
        pub const fn present_registration_descriptor_bytes(self) -> usize {
            self.present_registration_descriptor_bytes
        }
        pub const fn present_held_frame_descriptor_bytes(self) -> usize {
            self.present_held_frame_descriptor_bytes
        }
        pub const fn present_draining_gate_arc_allocation_bytes(self) -> usize {
            self.present_draining_gate_arc_allocation_bytes
        }
        pub const fn present_process_service_bytes(self) -> usize {
            self.present_process_service_bytes
        }
    }

    /// Return release `size_of` receipts for one head, one GPU gate, one
    /// registered/draining consumer and the process-wide render owners.
    /// No backend renderer census or budget live-byte counter is queried.
    pub fn scalar_receipt() -> ScalarReceipt {
        let present = client::render::backend::present::test_hooks::scalar_receipt();
        let process_budget = super::process_budget_scalar_receipt();
        let budgets_enabled = process_budget.state_allocation_count != 0;
        let head_inline_bytes = std::mem::size_of::<Head>() - std::mem::size_of::<Renderer>();
        let head_thread_local_bytes = super::head_thread_local_bytes(budgets_enabled);
        let head_wake_arc_allocation_bytes = crate::render::arc_allocation_bytes::<HeadWake>();
        let head_cell_arc_allocation_bytes = crate::render::head_cell_arc_allocation_bytes();
        let head_budget_state_allocation_count = if budgets_enabled {
            client::render::budget::RenderBudget::head_local_state_count()
        } else {
            0
        };
        let head_budget_arc_allocation_bytes = if budgets_enabled {
            client::render::budget::RenderBudget::head_local_state_allocation_bytes()
        } else {
            0
        };
        let s_head = head_inline_bytes
            + head_thread_local_bytes
            + head_wake_arc_allocation_bytes
            + head_cell_arc_allocation_bytes
            + head_budget_arc_allocation_bytes;
        let render_process_static_bytes = std::mem::size_of::<LazyLock<ProcessRender>>();
        let fair_process_static_bytes = super::fair_process_static_bytes();
        let render_service_arc_allocation_bytes = crate::render::service_arc_allocation_bytes();
        let present_process_service_bytes = present.process_service_bytes();
        let s_service = render_process_static_bytes
            + fair_process_static_bytes
            + render_service_arc_allocation_bytes
            + process_budget.state_allocation_bytes
            + present_process_service_bytes;
        let s_consumer = present.s_consumer();
        let present_draining_gate_arc_allocation_bytes = present.draining_gate_allocation_bytes();
        let s_consumer_active_bytes = s_consumer
            .checked_sub(present_draining_gate_arc_allocation_bytes)
            .expect("draining gate allocation is included in the consumer receipt");
        ScalarReceipt {
            s_head,
            s_gate: present.s_gate(),
            s_consumer_active_bytes,
            s_consumer,
            s_service,
            mailbox_per_loaded_bot_bytes: FrameBuf::fixed_storage_bytes(),
            head_inline_bytes,
            head_thread_local_bytes,
            head_wake_arc_allocation_bytes,
            head_cell_arc_allocation_bytes,
            head_budget_state_allocation_count,
            head_budget_arc_allocation_bytes,
            render_process_static_bytes,
            fair_process_static_bytes,
            render_service_arc_allocation_bytes,
            process_budget_state_allocation_count: process_budget.state_allocation_count,
            process_budget_arc_allocation_bytes: process_budget.state_allocation_bytes,
            present_gate_arc_allocation_bytes: present.gate_allocation_bytes(),
            present_retained_arc_allocation_bytes: present.retained_present_allocation_bytes(),
            present_registration_descriptor_bytes: present.registration_descriptor_bytes(),
            present_held_frame_descriptor_bytes: present.held_frame_descriptor_bytes(),
            present_draining_gate_arc_allocation_bytes,
            present_process_service_bytes,
        }
    }
}

struct Configuration {
    started: bool,
    #[cfg(feature = "render-test-hooks")]
    fixture: Option<test_hooks::ProcessFixture>,
    budget: Option<client::render::budget::RenderBudget>,
    #[cfg(feature = "render-test-hooks")]
    control: Option<Arc<dyn test_hooks::RenderTestControl>>,
}
static CONFIG: Mutex<Configuration> = Mutex::new(Configuration {
    started: false,
    #[cfg(feature = "render-test-hooks")]
    fixture: None,
    budget: None,
    #[cfg(feature = "render-test-hooks")]
    control: None,
});
static CALIBRATION: OnceLock<CalibrationConfig> = OnceLock::new();
static INITIALIZED: OnceLock<Option<CalibrationConfig>> = OnceLock::new();
static BUDGET: OnceLock<Option<client::render::budget::RenderBudget>> = OnceLock::new();

/// Explicit provisional settings for measuring fair scene admission.
///
/// Calibration is opt-in and does not change the production renderer's
/// default wrap. These values enable the real fair path only in a build with
/// `render-diagnostics` or `render-test-hooks`; choose measured values before
/// the process service or any render head is created.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CalibrationConfig {
    /// Maximum lifetime of an unconsumed GPU scene grant.
    pub grant_ttl: Duration,
    /// Maximum time a consumed scene lease may occupy the render workspace.
    pub scene_quantum: Duration,
}

/// Configure the provisional fair-render measurement path once, before any
/// process service or render head exists. Both durations use the render
/// service's finite, positive validation. This is not production activation.
pub fn configure_calibration(config: CalibrationConfig) -> Result<(), RenderError> {
    let process = lock(&CONFIG);
    if process.started || CALIBRATION.get().is_some() {
        return Err(RenderError::ConfigurationLocked);
    }
    validate_calibration(config)?;
    CALIBRATION
        .set(config)
        .map_err(|_| RenderError::ConfigurationLocked)
}
/// Configure explicit calibration memory bounds before the process service or
/// any render head is created. The limits use the same real render budget
/// source for diagnostics and test-hook builds.
pub fn configure_calibration_limits(
    limits: client::render::budget::RenderBudgetLimits,
) -> Result<(), RenderError> {
    let mut process = lock(&CONFIG);
    if process.started || process.budget.is_some() {
        return Err(RenderError::ConfigurationLocked);
    }
    process.budget = Some(client::render::budget::RenderBudget::new(limits));
    Ok(())
}

fn validate_calibration(config: CalibrationConfig) -> Result<(), RenderError> {
    let service = RenderService::new(config.grant_ttl)?;
    service.set_consumed_quantum(config.scene_quantum)
}

#[cfg(feature = "render-test-hooks")]
pub(super) fn configured_test_control() -> Option<Arc<dyn test_hooks::RenderTestControl>> {
    lock(&CONFIG).control.clone()
}

#[cfg(feature = "render-test-hooks")]
fn process_budget_scalar_receipt() -> client::render::budget::RenderBudgetScalarReceipt {
    let config = lock(&CONFIG);
    if let Some(budget) = config.budget.as_ref() {
        return budget.scalar_allocation_receipt(None);
    }
    if let Some(budget) = BUDGET.get().and_then(Option::as_ref) {
        return budget.scalar_allocation_receipt(None);
    }
    client::render::budget::RenderBudgetScalarReceipt::default()
}

#[cfg(feature = "render-test-hooks")]
fn fair_process_static_bytes() -> usize {
    std::mem::size_of::<Mutex<Configuration>>()
        + std::mem::size_of::<OnceLock<CalibrationConfig>>()
        + std::mem::size_of::<OnceLock<Option<CalibrationConfig>>>()
        + std::mem::size_of::<OnceLock<Option<client::render::budget::RenderBudget>>>()
}

#[cfg(feature = "render-test-hooks")]
fn head_thread_local_bytes(budgets_enabled: bool) -> usize {
    std::mem::size_of::<RefCell<Option<GpuFrame>>>()
        + 2 * std::mem::size_of::<Cell<bool>>()
        + std::mem::size_of::<Cell<Option<ReservationIdentity>>>()
        + std::mem::size_of::<RefCell<Option<(HeadIdentity, Arc<dyn test_hooks::RenderTestControl>)>>>(
        )
        + if budgets_enabled {
            client::render::budget::RenderBudget::thread_local_storage_bytes()
        } else {
            0
        }
}

pub(super) fn initialize() -> Option<CalibrationConfig> {
    *INITIALIZED.get_or_init(|| {
        let mut config = lock(&CONFIG);
        config.started = true;
        assert!(BUDGET.set(config.budget.take()).is_ok());
        CALIBRATION.get().copied()
    })
}

pub(super) fn head_budget() -> Option<client::render::budget::RenderBudget> {
    BUDGET
        .get()
        .and_then(Option::as_ref)
        .map(client::render::budget::RenderBudget::for_head)
}

pub(super) fn process_budget_scope() -> Option<client::render::budget::RenderBudgetScope> {
    BUDGET
        .get()
        .and_then(Option::as_ref)
        .map(client::render::budget::RenderBudget::scope)
}

/// Run real shared-client work under the configured process budget.
///
/// The process service is initialized first so its configured budget is
/// installed before the caller begins the bounded operation.
pub fn with_process_render_budget<T>(action: impl FnOnce() -> T) -> T {
    LazyLock::force(&PROCESS);
    let _scope = process_budget_scope();
    action()
}

pub(super) fn enabled() -> bool {
    CALIBRATION.get().is_some()
}

pub(super) fn heartbeat(now: Instant, deadline: Option<Deadline>) -> Option<Deadline> {
    let config = *CALIBRATION.get()?;
    let beat = Deadline::at(
        now.checked_add(config.scene_quantum)
            .expect("validated calibration quantum"),
    );
    Some(deadline.map_or(beat, |due| {
        if due.instant() < beat.instant() {
            due
        } else {
            beat
        }
    }))
}

/// One distinct ordinary due-scene opportunity, not a grant poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DueOpportunity(NonZeroU64);

/// One cross-thread grant delivery, measured at its four distinct boundaries.
#[derive(Debug, Clone, Copy)]
pub struct GrantLatency {
    reservation: ReservationIdentity,
    published_at: Instant,
    notified_at: Instant,
    woke_at: Instant,
    consumed_at: Instant,
}

impl GrantLatency {
    fn observed(
        reservation: ReservationIdentity,
        published_at: Instant,
        notified_at: Instant,
        woke_at: Instant,
        consumed_at: Instant,
    ) -> Option<Self> {
        notified_at.checked_duration_since(published_at)?;
        woke_at.checked_duration_since(notified_at)?;
        consumed_at.checked_duration_since(woke_at)?;
        Some(Self {
            reservation,
            published_at,
            notified_at,
            woke_at,
            consumed_at,
        })
    }

    pub const fn reservation(self) -> ReservationIdentity {
        self.reservation
    }
    pub const fn published_at(self) -> Instant {
        self.published_at
    }
    pub const fn notified_at(self) -> Instant {
        self.notified_at
    }
    pub const fn woke_at(self) -> Instant {
        self.woke_at
    }
    pub const fn consumed_at(self) -> Instant {
        self.consumed_at
    }
    pub fn publication_to_consume(self) -> Duration {
        self.consumed_at.duration_since(self.published_at)
    }
    pub fn publication_to_notice(self) -> Duration {
        self.notified_at.duration_since(self.published_at)
    }
    pub fn notice_to_wake(self) -> Duration {
        self.woke_at.duration_since(self.notified_at)
    }
    pub fn wake_to_consume(self) -> Duration {
        self.consumed_at.duration_since(self.woke_at)
    }
}

/// A fixed-size identity-checked sample. No renderer container traversal occurs here.
#[derive(Debug, Clone, Copy)]
pub struct HeadFairStats {
    identity: HeadIdentity,
    due_skips: u64,
    max_due_skips: u64,
    skipped_due_opportunities: u64,
    expiry_streak: u64,
    max_expiry_streak: u64,
    expiry_events: u64,
    busy_paints: u64,
    busy_paint_time: Duration,
    granted_paints: u64,
    granted_paint_time: Duration,
    scene_consumes: u64,
    last_scene_consume_at: Option<Instant>,
    max_scene_consume_gap: Duration,
    scene_gap_window_start: Option<Instant>,
    max_window_scene_consume_gap: Duration,
    grant_delivery_samples: u64,
    max_grant_to_consume: Duration,
    max_publication_to_notice: Duration,
    max_notice_to_wake: Duration,
    max_wake_to_consume: Duration,
    grant_wake_samples: u64,
    last_grant_latency: Option<GrantLatency>,
    max_grant_latency: Option<GrantLatency>,
    scene_releases: u64,
    scene_lease_time: Duration,
    max_scene_lease_time: Duration,
    last_opportunity: Option<DueOpportunity>,
    last_expired: Option<ReservationIdentity>,
    oldest_demand_age: Duration,
    render_focused: Option<bool>,
    focus_applications: u64,
    backend: BackendKind,
    has_valid_scene: bool,
    scene_progress: SceneProgress,
    retained_storage_allocations: u64,
    retained_storage_bytes: usize,
}

impl HeadFairStats {
    pub fn identity(&self) -> HeadIdentity {
        self.identity
    }
    pub fn due_skips(&self) -> u64 {
        self.due_skips
    }
    pub fn max_due_skips(&self) -> u64 {
        self.max_due_skips
    }
    pub fn skipped_due_opportunities(&self) -> u64 {
        self.skipped_due_opportunities
    }
    pub fn expiry_streak(&self) -> u64 {
        self.expiry_streak
    }
    pub fn max_expiry_streak(&self) -> u64 {
        self.max_expiry_streak
    }
    pub fn expiry_events(&self) -> u64 {
        self.expiry_events
    }
    pub fn busy_paints(&self) -> u64 {
        self.busy_paints
    }
    pub fn busy_paint_time(&self) -> Duration {
        self.busy_paint_time
    }
    pub fn granted_paints(&self) -> u64 {
        self.granted_paints
    }
    pub fn granted_paint_time(&self) -> Duration {
        self.granted_paint_time
    }
    pub fn scene_consumes(&self) -> u64 {
        self.scene_consumes
    }
    pub fn last_scene_consume_at(&self) -> Option<Instant> {
        self.last_scene_consume_at
    }
    pub fn max_scene_consume_gap(&self) -> Duration {
        self.max_scene_consume_gap
    }
    pub fn scene_gap_window_start(&self) -> Option<Instant> {
        self.scene_gap_window_start
    }
    pub fn max_window_scene_consume_gap(&self) -> Duration {
        self.max_window_scene_consume_gap
    }

    pub(crate) fn begin_scene_gap_window(&mut self, start: Instant) {
        self.scene_gap_window_start = Some(start);
        self.max_window_scene_consume_gap = Duration::ZERO;
    }

    pub(crate) fn inherit_scene_gap_window(&mut self, previous: Self) {
        self.scene_gap_window_start = previous.scene_gap_window_start;
        self.max_window_scene_consume_gap = previous.max_window_scene_consume_gap;
        let Some(start) = self.scene_gap_window_start else {
            return;
        };
        if self.scene_consumes == previous.scene_consumes {
            return;
        }
        if let Some(consumed_at) = self.last_scene_consume_at {
            let previous_at = previous.last_scene_consume_at.unwrap_or(start).max(start);
            self.max_window_scene_consume_gap = self
                .max_window_scene_consume_gap
                .max(consumed_at.saturating_duration_since(previous_at));
        }
    }

    pub fn grant_delivery_samples(&self) -> u64 {
        self.grant_delivery_samples
    }
    pub fn max_grant_to_consume(&self) -> Duration {
        self.max_grant_to_consume
    }
    pub fn max_publication_to_notice(&self) -> Duration {
        self.max_publication_to_notice
    }
    pub fn max_notice_to_wake(&self) -> Duration {
        self.max_notice_to_wake
    }
    pub fn max_wake_to_consume(&self) -> Duration {
        self.max_wake_to_consume
    }
    pub fn grant_wake_samples(&self) -> u64 {
        self.grant_wake_samples
    }
    pub fn last_grant_latency(&self) -> Option<GrantLatency> {
        self.last_grant_latency
    }
    pub fn max_grant_latency(&self) -> Option<GrantLatency> {
        self.max_grant_latency
    }
    pub fn render_focused(&self) -> Option<bool> {
        self.render_focused
    }
    pub fn focus_applications(&self) -> u64 {
        self.focus_applications
    }
    pub fn scene_releases(&self) -> u64 {
        self.scene_releases
    }
    pub fn scene_lease_time(&self) -> Duration {
        self.scene_lease_time
    }
    pub fn max_scene_lease_time(&self) -> Duration {
        self.max_scene_lease_time
    }
    pub fn oldest_demand_age(&self) -> Duration {
        self.oldest_demand_age
    }
    pub fn backend(&self) -> BackendKind {
        self.backend
    }
    pub fn has_valid_scene(&self) -> bool {
        self.has_valid_scene
    }
    pub fn scene_progress(&self) -> SceneProgress {
        self.scene_progress
    }
    pub fn retained_storage_allocations(&self) -> u64 {
        self.retained_storage_allocations
    }
    pub fn retained_storage_bytes(&self) -> usize {
        self.retained_storage_bytes
    }
}

#[derive(Default)]
struct SceneDemand {
    due_at: Option<Deadline>,
    reservation: Option<ReservationIdentity>,
}

pub(super) struct FairHead {
    demand: SceneDemand,
    stats: HeadFairStats,
    next_opportunity: Option<NonZeroU64>,
    opportunity: Option<DueOpportunity>,
    focused: Option<bool>,
    #[cfg(feature = "render-test-hooks")]
    control: Option<Arc<dyn test_hooks::RenderTestControl>>,
    opportunity_exhausted: bool,
    woke_at: Option<Instant>,
}

impl FairHead {
    #[cfg(feature = "render-test-hooks")]
    pub(super) fn observe_client_cycle(
        &self,
        slot: SlotKey,
        backend: BackendKind,
        cycle: i32,
        complete_game_scene: bool,
    ) {
        if let Some(control) = &self.control {
            control.client_cycle(slot, backend, cycle, complete_game_scene);
        }
    }

    pub(super) fn new(identity: HeadIdentity, backend: BackendKind) -> Self {
        Self {
            demand: SceneDemand::default(),
            stats: HeadFairStats {
                identity,
                due_skips: 0,
                max_due_skips: 0,
                skipped_due_opportunities: 0,
                expiry_streak: 0,
                max_expiry_streak: 0,
                expiry_events: 0,
                busy_paints: 0,
                busy_paint_time: Duration::ZERO,
                granted_paints: 0,
                granted_paint_time: Duration::ZERO,
                scene_consumes: 0,
                last_scene_consume_at: None,
                max_scene_consume_gap: Duration::ZERO,
                scene_gap_window_start: None,
                max_window_scene_consume_gap: Duration::ZERO,
                grant_delivery_samples: 0,
                max_grant_to_consume: Duration::ZERO,
                max_publication_to_notice: Duration::ZERO,
                max_notice_to_wake: Duration::ZERO,
                max_wake_to_consume: Duration::ZERO,
                grant_wake_samples: 0,
                last_grant_latency: None,
                max_grant_latency: None,
                scene_releases: 0,
                scene_lease_time: Duration::ZERO,
                max_scene_lease_time: Duration::ZERO,
                last_opportunity: None,
                last_expired: None,
                oldest_demand_age: Duration::ZERO,
                render_focused: Some(false),
                focus_applications: 0,
                backend,
                has_valid_scene: false,
                scene_progress: SceneProgress::NoScene,
                retained_storage_allocations: 0,
                retained_storage_bytes: 0,
            },
            next_opportunity: NonZeroU64::new(1),
            opportunity: None,
            focused: Some(false),
            #[cfg(feature = "render-test-hooks")]
            control: None,
            opportunity_exhausted: false,
            woke_at: None,
        }
    }
    #[cfg(feature = "render-test-hooks")]
    pub(super) fn with_control(
        mut self,
        control: Option<Arc<dyn test_hooks::RenderTestControl>>,
    ) -> Self {
        self.control = control;
        self
    }

    fn refresh(&mut self, snapshot: crate::render::HeadSnapshot, now: Instant) {
        if snapshot.state() == ReservationState::Expired
            && snapshot.reservation() != self.stats.last_expired
        {
            self.stats.last_expired = snapshot.reservation();
            self.stats.expiry_events = self.stats.expiry_events.saturating_add(1);
            self.stats.expiry_streak = self.stats.expiry_streak.saturating_add(1);
            self.stats.max_expiry_streak =
                self.stats.max_expiry_streak.max(self.stats.expiry_streak);
        }
        match snapshot.state() {
            ReservationState::Pending | ReservationState::Granted | ReservationState::Expired => {
                self.demand.due_at = snapshot.due_at();
                self.demand.reservation = snapshot.reservation();
            }
            ReservationState::Cancelled => self.demand = SceneDemand::default(),
            ReservationState::Idle | ReservationState::Consumed => {}
        }
        self.stats.oldest_demand_age = if matches!(
            snapshot.state(),
            ReservationState::Pending | ReservationState::Granted
        ) {
            snapshot.due_at().map_or(Duration::ZERO, |due| {
                now.saturating_duration_since(due.instant())
            })
        } else {
            Duration::ZERO
        };
    }

    fn note_due(&mut self, now: Instant) -> Result<(), RenderError> {
        let Some(next) = self.next_opportunity else {
            self.opportunity_exhausted = true;
            return Err(RenderError::ReservationIdExhausted);
        };
        self.next_opportunity = next.get().checked_add(1).and_then(NonZeroU64::new);
        self.opportunity = Some(DueOpportunity(next));
        if self.demand.due_at.is_none() {
            self.demand.due_at = Some(Deadline::at(now));
        }
        Ok(())
    }
}

impl Head {
    pub(crate) fn slot_woke(&mut self, now: Instant) {
        if enabled() {
            self.fair.woke_at = Some(now);
        }
    }

    pub(crate) fn reconcile_scene_eligibility(
        &mut self,
        can_scene: bool,
        focused: bool,
        now: Instant,
    ) {
        if !enabled() || self.backend_kind() != BackendKind::Gpu {
            return;
        }
        let Some(lease) = self.lease.as_ref() else {
            return;
        };
        if self.fair.focused != Some(focused) {
            match PROCESS.service.set_focused(lease, focused, now) {
                Ok(()) => {
                    self.fair.focused = Some(focused);
                    self.fair.stats.focus_applications =
                        self.fair.stats.focus_applications.saturating_add(1);
                }
                Err(error) => report(Err(error)),
            }
        }
        self.fair.stats.render_focused = self.fair.focused;
        match PROCESS.service.poll(lease, now) {
            Ok((_, snapshot)) => {
                self.fair.refresh(snapshot, now);
                if !can_scene {
                    if matches!(
                        snapshot.state(),
                        ReservationState::Pending | ReservationState::Granted
                    ) {
                        report(PROCESS.service.cancel_request(lease, now));
                    }
                    self.fair.demand = SceneDemand::default();
                    self.fair.opportunity = None;
                    self.fair.stats.oldest_demand_age = Duration::ZERO;
                }
            }
            Err(error) => report(Err(error)),
        }
        self.output.publish_fair_stats(self.fair.stats);
    }

    pub(crate) fn has_grant(&self, now: Instant) -> bool {
        enabled()
            && self.lease.as_ref().is_some_and(|lease| {
                let snapshot = lease.snapshot().1;
                snapshot.state() == ReservationState::Granted
                    && snapshot
                        .expires_at()
                        .is_some_and(|expiry| now < expiry.instant())
            })
    }

    pub(crate) fn note_due(&mut self, now: Instant) -> Result<(), RenderError> {
        if !enabled() || self.backend_kind() != BackendKind::Gpu {
            return Ok(());
        }
        self.fair.note_due(now)
    }
}

pub(super) fn retire(head: &mut Head) {
    if enabled() {
        head.fair.demand = SceneDemand::default();
        head.fair.opportunity = None;
        head.fair.stats.due_skips = 0;
        head.fair.stats.expiry_streak = 0;
        head.fair.stats.oldest_demand_age = Duration::ZERO;
        head.output.publish_fair_stats(head.fair.stats);
    }
}

thread_local! {
    static HOST_ACTIVE: Cell<bool> = const { Cell::new(false) };
    #[cfg(feature = "render-test-hooks")]
    static OWNS_LEASE: Cell<bool> = const { Cell::new(false) };
    #[cfg(feature = "render-test-hooks")]
    static RESERVATION: Cell<Option<ReservationIdentity>> = const { Cell::new(None) };
    #[cfg(feature = "render-test-hooks")]
    static CONTROL: RefCell<Option<(HeadIdentity, Arc<dyn test_hooks::RenderTestControl>)>> = const { RefCell::new(None) };
}

struct HostPaintScope {
    previous: bool,
    #[cfg(feature = "render-test-hooks")]
    previous_control: Option<(HeadIdentity, Arc<dyn test_hooks::RenderTestControl>)>,
    #[cfg(feature = "render-test-hooks")]
    previous_lease: bool,
    #[cfg(feature = "render-test-hooks")]
    previous_reservation: Option<ReservationIdentity>,
    #[cfg(feature = "render-test-hooks")]
    _thread_bound: PhantomData<Rc<()>>,
}
impl HostPaintScope {
    fn enter(_head: &Head) -> Self {
        Self {
            previous: HOST_ACTIVE.replace(true),
            #[cfg(feature = "render-test-hooks")]
            previous_control: CONTROL.replace(
                _head
                    .fair
                    .control
                    .as_ref()
                    .map(|control| (_head.identity, Arc::clone(control))),
            ),
            #[cfg(feature = "render-test-hooks")]
            previous_lease: OWNS_LEASE.replace(false),
            #[cfg(feature = "render-test-hooks")]
            previous_reservation: RESERVATION.replace(None),
            #[cfg(feature = "render-test-hooks")]
            _thread_bound: PhantomData,
        }
    }
}
impl Drop for HostPaintScope {
    fn drop(&mut self) {
        HOST_ACTIVE.set(self.previous);
        #[cfg(feature = "render-test-hooks")]
        {
            CONTROL.replace(self.previous_control.take());
            OWNS_LEASE.set(self.previous_lease);
            RESERVATION.set(self.previous_reservation);
        }
    }
}

pub(super) fn host_active() -> bool {
    HOST_ACTIVE.get()
}

#[cfg(feature = "render-test-hooks")]
fn event(point: test_hooks::RenderTestPoint) {
    CONTROL.with(|cell| {
        if let Some((head, control)) = cell.borrow().as_ref() {
            control.at(test_hooks::RenderTestEvent {
                head: *head,
                point,
                owns_lease: OWNS_LEASE.get(),
                reservation: RESERVATION.get(),
            });
        }
    });
}

#[cfg(feature = "render-test-hooks")]
fn bridge(point: client::render::backend::gpu::test_hooks::SceneTestPoint) {
    use client::render::backend::gpu::test_hooks::SceneTestPoint;
    match point {
        SceneTestPoint::AfterAdmission | SceneTestPoint::VertexCoordinateOverflow => {}
        SceneTestPoint::BeforeTerminal => event(test_hooks::RenderTestPoint::BeforeTerminal),
        SceneTestPoint::OverlayEntry => event(test_hooks::RenderTestPoint::OverlayEntry),
    }
}

#[cfg(feature = "render-test-hooks")]
pub(super) struct LoadingPaintScope<'a> {
    client: &'a mut Client,
    previous: Option<i32>,
    previous_cross: Option<(i32, i32, i32, i32)>,
}
#[cfg(feature = "render-test-hooks")]
impl<'a> LoadingPaintScope<'a> {
    pub(super) fn enter(client: &'a mut Client, loading: bool) -> Self {
        let previous = (loading && client.ingame).then_some(client.scene_state);
        if previous.is_some() {
            client.scene_state = 1;
        }
        Self {
            client,
            previous,
            previous_cross: None,
        }
    }
    fn cross_paint(&mut self, input: Option<test_hooks::RenderTestCross>) {
        let Some(input) = input.filter(|_| self.client.ingame) else {
            return;
        };
        self.previous_cross = Some((
            self.client.cross_mode,
            self.client.cross_cycle,
            self.client.cross_x,
            self.client.cross_y,
        ));
        self.client.cross_cycle = 0;
        match input {
            test_hooks::RenderTestCross::Hidden => self.client.cross_mode = 0,
            test_hooks::RenderTestCross::At { x, y } => {
                self.client.cross_mode = 1;
                self.client.cross_x = x;
                self.client.cross_y = y;
            }
        }
    }
    fn client(&mut self) -> &mut Client {
        self.client
    }
}
#[cfg(feature = "render-test-hooks")]
impl Drop for LoadingPaintScope<'_> {
    fn drop(&mut self) {
        if let Some(previous) = self.previous {
            self.client.scene_state = previous;
        }
        if let Some((mode, cycle, x, y)) = self.previous_cross {
            self.client.cross_mode = mode;
            self.client.cross_cycle = cycle;
            self.client.cross_x = x;
            self.client.cross_y = y;
        }
    }
}

pub(super) struct FairFrame {
    head: FairHead,
    lease: Option<FrameLease>,
    consumed_at: Option<Instant>,
}

struct FairLent<'a> {
    lease: &'a mut Option<HeadLease>,
    scenes: &'a mut u64,
    state: &'a mut FairHead,
}
impl<'a> FairLent<'a> {
    fn lend(
        lease: &'a mut Option<HeadLease>,
        scenes: &'a mut u64,
        state: &'a mut FairHead,
        wake: &Arc<HeadWake>,
    ) -> Self {
        let empty = FairHead::new(state.stats.identity, state.stats.backend);
        let lent_state = std::mem::replace(state, empty);
        let frame = GpuFrame {
            lease: lease.take().expect("attached head owns its lease"),
            wake: Arc::clone(wake),
            scenes: *scenes,
            fair: Some(FairFrame {
                head: lent_state,
                lease: None,
                consumed_at: None,
            }),
        };
        GPU_FRAME.with(|cell| {
            assert!(
                cell.borrow().is_none(),
                "a slot cannot recursively lend a render head"
            );
            *cell.borrow_mut() = Some(frame);
        });
        Self {
            lease,
            scenes,
            state,
        }
    }
}
impl Drop for FairLent<'_> {
    fn drop(&mut self) {
        if let Some(mut frame) = GPU_FRAME.with(|cell| cell.borrow_mut().take()) {
            let mut fair = frame.fair.take().expect("fair lend has fair state");
            // An unwind returns shared client guards before it reaches this owner.
            drop(fair.lease.take());
            *self.lease = Some(frame.lease);
            *self.scenes = frame.scenes;
            *self.state = fair.head;
        }
    }
}

pub(super) fn paint(head: &mut Head, client: &mut Client) -> FrameOutput {
    let _scope = HostPaintScope::enter(head);
    #[cfg(feature = "render-test-hooks")]
    let _observer = client::render::backend::gpu::test_hooks::install_for_thread(bridge);
    #[cfg(feature = "render-test-hooks")]
    let failure = head
        .fair
        .control
        .as_ref()
        .and_then(|control| control.resource_failure());
    #[cfg(feature = "render-test-hooks")]
    let _failure = failure.map(client::render::backend::gpu::test_hooks::resource_failure_at);
    #[cfg(feature = "render-test-hooks")]
    let loading = head
        .fair
        .control
        .as_ref()
        .is_some_and(|control| control.loading_paint());
    #[cfg(feature = "render-test-hooks")]
    let mut loading_scope = LoadingPaintScope::enter(client, loading);
    #[cfg(feature = "render-test-hooks")]
    loading_scope.cross_paint(
        head.fair
            .control
            .as_ref()
            .and_then(|control| control.cross_paint()),
    );
    #[cfg(feature = "render-test-hooks")]
    let client = loading_scope.client();

    let start = Instant::now();
    let consumes = head.fair.stats.scene_consumes;
    let output = if head.backend_kind() == BackendKind::Gpu {
        let _lent = FairLent::lend(
            &mut head.lease,
            &mut head.scenes,
            &mut head.fair,
            &head.wake,
        );
        head.renderer.mainredraw(&mut *client)
    } else {
        head.renderer.mainredraw(&mut *client)
    };
    let client_scene_state = client.scene_state;
    let elapsed = start.elapsed();
    if head.backend_kind() == BackendKind::Gpu {
        if head.fair.stats.scene_consumes > consumes {
            head.fair.stats.granted_paints = head.fair.stats.granted_paints.saturating_add(1);
            head.fair.stats.granted_paint_time =
                head.fair.stats.granted_paint_time.saturating_add(elapsed);
        } else if client_scene_state == 2 {
            head.fair.stats.busy_paints = head.fair.stats.busy_paints.saturating_add(1);
            head.fair.stats.busy_paint_time =
                head.fair.stats.busy_paint_time.saturating_add(elapsed);
            PROCESS.counters.busy.fetch_add(1, Ordering::Relaxed);
        }
        if let Some(opportunity) = head.fair.opportunity.take() {
            if head.fair.stats.last_opportunity != Some(opportunity) {
                head.fair.stats.last_opportunity = Some(opportunity);
                if head.fair.stats.scene_consumes == consumes {
                    head.fair.stats.due_skips = head.fair.stats.due_skips.saturating_add(1);
                    head.fair.stats.max_due_skips =
                        head.fair.stats.max_due_skips.max(head.fair.stats.due_skips);
                    head.fair.stats.skipped_due_opportunities =
                        head.fair.stats.skipped_due_opportunities.saturating_add(1);
                }
            }
        }
    }
    head.fair.stats.has_valid_scene = head.renderer.has_valid_scene();
    head.fair.stats.scene_progress = head.renderer.scene_progress();
    head.fair.stats.retained_storage_allocations = head.renderer.retained_storage_allocations();
    head.fair.stats.retained_storage_bytes = head.renderer.retained_storage_bytes();
    head.output.publish_fair_stats(head.fair.stats);
    output
}

pub(super) fn admission() -> SceneAdmission {
    GPU_FRAME.with(|cell| {
        let mut frame = cell.borrow_mut();
        let Some(frame) = frame.as_mut() else {
            return SceneAdmission::Standalone;
        };
        #[cfg(feature = "render-test-hooks")]
        let now = frame
            .fair
            .as_ref()
            .and_then(|fair| fair.head.control.as_ref())
            .map_or_else(Instant::now, |control| control.now());
        #[cfg(not(feature = "render-test-hooks"))]
        let now = Instant::now();
        admit(frame, &PROCESS.service, now)
    })
}

fn admit(frame: &mut GpuFrame, service: &RenderService, now: Instant) -> SceneAdmission {
    let Some(fair) = frame.fair.as_mut() else {
        return SceneAdmission::Standalone;
    };
    if fair.head.opportunity_exhausted {
        return SceneAdmission::Retained;
    }
    let due = fair.head.demand.due_at.unwrap_or_else(|| Deadline::at(now));
    let request = service.request_frame(&frame.lease, due, now);
    let (grant_sequence, grant_snapshot) = frame.lease.snapshot();
    fair.head.refresh(grant_snapshot, now);
    let identity = match request {
        Ok(RequestOutcome::Busy(identity)) => {
            fair.head.demand.reservation = Some(identity);
            return SceneAdmission::Retained;
        }
        Ok(RequestOutcome::Granted(identity)) => identity,
        Err(error) => {
            report(Err(error));
            return SceneAdmission::Retained;
        }
    };
    match service.try_consume(&frame.lease, identity, now) {
        Ok(lease) => {
            let consumed_at = Instant::now();
            let stats = &mut fair.head.stats;
            if let Some(previous) = stats.last_scene_consume_at.replace(consumed_at) {
                stats.max_scene_consume_gap = stats
                    .max_scene_consume_gap
                    .max(consumed_at.saturating_duration_since(previous));
            }
            if let Some(elapsed) = grant_snapshot
                .granted_at()
                .and_then(|published_at| consumed_at.checked_duration_since(published_at))
            {
                fair.head.stats.grant_delivery_samples =
                    fair.head.stats.grant_delivery_samples.saturating_add(1);
                fair.head.stats.max_grant_to_consume =
                    fair.head.stats.max_grant_to_consume.max(elapsed);
            }
            if let Some(latency) = grant_snapshot
                .granted_at()
                .zip(frame.wake.grant_notice(grant_sequence))
                .zip(fair.head.woke_at)
                .and_then(|((published, notified), woke)| {
                    GrantLatency::observed(identity, published, notified, woke, consumed_at)
                })
            {
                let stats = &mut fair.head.stats;
                stats.grant_wake_samples = stats.grant_wake_samples.saturating_add(1);
                stats.last_grant_latency = Some(latency);
                stats.max_publication_to_notice = stats
                    .max_publication_to_notice
                    .max(latency.publication_to_notice());
                stats.max_notice_to_wake = stats.max_notice_to_wake.max(latency.notice_to_wake());
                stats.max_wake_to_consume =
                    stats.max_wake_to_consume.max(latency.wake_to_consume());
                if stats.max_grant_latency.is_none_or(|previous| {
                    latency.publication_to_consume() > previous.publication_to_consume()
                }) {
                    stats.max_grant_latency = Some(latency);
                }
            }
            fair.consumed_at = Some(consumed_at);
            fair.lease = Some(lease);
            fair.head.demand = SceneDemand::default();
            fair.head.stats.due_skips = 0;
            fair.head.stats.expiry_streak = 0;
            fair.head.stats.scene_consumes = fair.head.stats.scene_consumes.saturating_add(1);
            fair.head.stats.oldest_demand_age = Duration::ZERO;
            #[cfg(feature = "render-test-hooks")]
            {
                OWNS_LEASE.set(true);
                RESERVATION.set(Some(identity));
                event(test_hooks::RenderTestPoint::AfterConsume);
            }
            SceneAdmission::Rebuild
        }
        Err(error) => {
            fair.head.refresh(frame.lease.snapshot().1, now);
            match error {
                RenderError::Expired
                | RenderError::Cancelled
                | RenderError::StaleReservation
                | RenderError::NotGranted => {}
                _ => report(Err(error)),
            }
            SceneAdmission::Retained
        }
    }
}

pub(super) fn submit(frame: &mut GpuFrame, submit: &mut dyn FnMut()) {
    let fair = frame.fair.as_mut().expect("fair callback owns fair state");
    let Some(lease) = fair.lease.take() else {
        return;
    };
    submit();
    #[cfg(feature = "render-test-hooks")]
    let now = fair
        .head
        .control
        .as_ref()
        .map_or_else(Instant::now, |control| control.now());
    #[cfg(not(feature = "render-test-hooks"))]
    let now = Instant::now();
    match client::render::backend::gpu::current_scene_submit_result() {
        SceneSubmit::Submitted => {
            PROCESS.counters.leased(None);
            frame.scenes = frame.scenes.saturating_add(1);
            report(lease.publish_retained_scene(SceneHandle::from_opaque(frame.scenes)));
        }
        SceneSubmit::LockContended => report(PROCESS.service.record_fault(
            lease.identity(),
            crate::render::RenderFaultKind::LockContention,
            now,
        )),
        SceneSubmit::ResourceLimited => report(PROCESS.service.record_fault(
            lease.identity(),
            crate::render::RenderFaultKind::ResourceLimit,
            now,
        )),
        SceneSubmit::NotRun => host_log!(
            Category::Lifecycle,
            Level::Warn,
            "render callback did not submit the consumed scene"
        ),
    }
    #[cfg(feature = "render-test-hooks")]
    event(test_hooks::RenderTestPoint::BeforeLeaseRelease);
    #[cfg(feature = "render-test-hooks")]
    let finished_at = fair
        .head
        .control
        .as_ref()
        .map_or_else(Instant::now, |control| control.now());
    #[cfg(not(feature = "render-test-hooks"))]
    let finished_at = Instant::now();
    report(lease.finish(finished_at));
    if let Some(consumed_at) = fair.consumed_at.take() {
        let elapsed = consumed_at.elapsed();
        let stats = &mut fair.head.stats;
        stats.scene_releases = stats.scene_releases.saturating_add(1);
        stats.scene_lease_time = stats.scene_lease_time.saturating_add(elapsed);
        stats.max_scene_lease_time = stats.max_scene_lease_time.max(elapsed);
    }
    #[cfg(feature = "render-test-hooks")]
    {
        OWNS_LEASE.set(false);
        RESERVATION.set(None);
        event(test_hooks::RenderTestPoint::AfterLeaseRelease);
    }
}

fn report(result: Result<(), RenderError>) {
    if let Err(error) = result {
        host_log!(Category::Lifecycle, Level::Warn, "fair render: {error}");
    }
}

#[cfg(all(test, feature = "render-test-hooks"))]
mod tests {
    use std::panic::{catch_unwind, AssertUnwindSafe};

    use super::*;

    fn local_heads() -> (RenderService, RenderPumpLease, HeadLease) {
        let service = RenderService::new(Duration::from_millis(20)).unwrap();
        let pump = service.register_pump().unwrap();
        let head = service
            .attach(
                &pump,
                SlotKey::from_incarnation(1).unwrap(),
                Arc::new(HeadWake::default()),
            )
            .unwrap();
        (service, pump, head)
    }

    #[test]
    fn retired_lent_head_never_submits_unleased() {
        let (service, _pump, lease) = local_heads();
        let identity = lease.identity();
        let mut frame = GpuFrame {
            lease,
            wake: Arc::new(HeadWake::default()),
            scenes: 0,
            fair: Some(FairFrame {
                head: FairHead::new(identity, BackendKind::Gpu),
                lease: None,
                consumed_at: None,
            }),
        };
        let now = Instant::now();
        service.retire(&mut frame.lease, now).unwrap();
        assert_eq!(admit(&mut frame, &service, now), SceneAdmission::Retained);
        assert!(frame.fair.as_ref().unwrap().lease.is_none());
        assert_eq!(frame.fair.as_ref().unwrap().head.stats.scene_consumes(), 0);
        assert_eq!(frame.scenes, 0);
    }

    #[test]
    fn scene_consume_gap_window_excludes_warmup_without_hiding_starvation() {
        let (_service, _pump, lease) = local_heads();
        let output = FrameBuf::new();
        let origin = Instant::now();
        let mut sample = FairHead::new(lease.identity(), BackendKind::Gpu).stats;
        sample.scene_consumes = 1;
        sample.last_scene_consume_at = Some(origin);
        sample.max_scene_consume_gap = Duration::from_secs(5);
        output.attach_head(lease.identity());
        output.publish_fair_stats(sample);
        assert!(output.begin_scene_gap_window(origin + Duration::from_secs(6)));
        output.publish_fair_stats(sample);
        let window = output.fair_stats().unwrap();
        assert_eq!(window.last_scene_consume_at(), Some(origin));
        assert_eq!(window.max_scene_consume_gap(), Duration::from_secs(5));
        assert_eq!(window.max_window_scene_consume_gap(), Duration::ZERO);

        sample.scene_consumes = 2;
        sample.last_scene_consume_at = Some(origin + Duration::from_secs(7));
        output.publish_fair_stats(sample);
        assert_eq!(
            output.fair_stats().unwrap().max_window_scene_consume_gap(),
            Duration::from_secs(1)
        );
        sample.scene_consumes = 3;
        sample.last_scene_consume_at = Some(origin + Duration::from_millis(9501));
        output.publish_fair_stats(sample);
        assert_eq!(
            output.fair_stats().unwrap().max_window_scene_consume_gap(),
            Duration::from_millis(2501)
        );
        assert!(output.begin_scene_gap_window(origin + Duration::from_secs(10)));
        output.publish_fair_stats(sample);
        assert_eq!(
            output.fair_stats().unwrap().max_window_scene_consume_gap(),
            Duration::ZERO
        );
        assert_eq!(
            output.fair_stats().unwrap().last_scene_consume_at(),
            sample.last_scene_consume_at()
        );
    }

    #[test]
    fn fair_stats_follow_head_generation_not_username() {
        let (service, pump, mut old) = local_heads();
        let output = FrameBuf::new();
        let mut previous = FairHead::new(old.identity(), BackendKind::Gpu).stats;
        previous.due_skips = 3;
        previous.max_due_skips = 3;
        output.attach_head(old.identity());
        output.publish_fair_stats(previous);
        assert_eq!(output.fair_stats().unwrap().max_due_skips(), 3);
        service.retire(&mut old, Instant::now()).unwrap();
        output.release_head(old.identity());
        let current = service
            .attach(
                &pump,
                SlotKey::from_incarnation(1).unwrap(),
                Arc::new(HeadWake::default()),
            )
            .unwrap();
        assert_ne!(current.identity(), old.identity());
        output.attach_head(current.identity());
        assert!(output.fair_stats().is_none());
        output.publish_fair_stats(previous);
        assert!(
            output.fair_stats().is_none(),
            "the old generation cannot replace the sample"
        );
        let fresh = FairHead::new(current.identity(), BackendKind::Gpu).stats;
        output.publish_fair_stats(fresh);
        assert_eq!(output.fair_stats().unwrap().identity(), current.identity());
        assert_eq!(output.fair_stats().unwrap().max_due_skips(), 0);
        output.publish_fair_stats(previous);
        assert_eq!(output.fair_stats().unwrap().identity(), current.identity());
    }

    #[test]
    fn due_opportunity_exhaustion_never_wraps_or_reuses_an_identity() {
        let (_, _, lease) = local_heads();
        let mut head = FairHead::new(lease.identity(), BackendKind::Gpu);
        head.next_opportunity = NonZeroU64::new(u64::MAX);
        let now = Instant::now();
        head.note_due(now).unwrap();
        assert_eq!(head.opportunity.unwrap().0.get(), u64::MAX);
        assert_eq!(head.next_opportunity, None);
        assert_eq!(head.note_due(now), Err(RenderError::ReservationIdExhausted));
        assert!(head.opportunity_exhausted);
        assert_eq!(head.opportunity.unwrap().0.get(), u64::MAX);
    }

    #[test]
    fn host_paint_scope_restores_tls_after_unwind() {
        let (wake, _park) = crate::wake_channel();
        let render = SlotRender::new(
            SlotKey::from_incarnation(999_990).unwrap(),
            FrameBuf::new(),
            Arc::new(wake),
        );
        let head = Head::attach(&render, true, true).unwrap();
        let now = Instant::now();
        let lease = head.lease.as_ref().unwrap();
        let RequestOutcome::Granted(reservation) = PROCESS
            .service
            .request_frame(lease, Deadline::at(now), now)
            .unwrap()
        else {
            panic!("the isolated unit-test service grants the CPU head");
        };
        assert!(!host_active());
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _scope = HostPaintScope::enter(&head);
            let _frame = PROCESS
                .service
                .try_consume(lease, reservation, now)
                .unwrap();
            OWNS_LEASE.set(true);
            RESERVATION.set(Some(reservation));
            assert!(host_active());
            panic!("unwind a live host paint scope");
        }));
        assert!(result.is_err());
        assert!(!host_active());
        assert!(!OWNS_LEASE.get());
        assert!(RESERVATION.get().is_none());
        assert!(CONTROL.with(|control| control.borrow().is_none()));
        assert!(GPU_FRAME.with(|frame| frame.borrow().is_none()));
    }

    #[test]
    fn loading_paint_scope_restores_client_state_after_unwind() {
        let mut client = Client::new_with_revision(
            client::client::ClientConfig {
                host: "127.0.0.1".into(),
                port: 43594,
                cache_dir: "/nonexistent-t44-cache".into(),
                members: true,
                lowmem: true,
            },
            client::client::ClientRevision::R289,
        );
        client.ingame = true;
        client.scene_state = 2;
        client.cross_mode = 2;
        client.cross_cycle = 100;
        client.cross_x = 7;
        client.cross_y = 8;
        {
            let mut scope = LoadingPaintScope::enter(&mut client, true);
            scope.cross_paint(Some(test_hooks::RenderTestCross::At { x: 300, y: 180 }));
            assert_eq!(scope.client().cross_mode, 1);
            assert_eq!(scope.client().cross_cycle, 0);
            assert_eq!((scope.client().cross_x, scope.client().cross_y), (300, 180));
            assert_eq!(scope.client().scene_state, 1);
        }
        assert_eq!(client.scene_state, 2);
        assert_eq!(
            (
                client.cross_mode,
                client.cross_cycle,
                client.cross_x,
                client.cross_y
            ),
            (2, 100, 7, 8),
        );
        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut scope = LoadingPaintScope::enter(&mut client, true);
            scope.cross_paint(Some(test_hooks::RenderTestCross::Hidden));
            assert_eq!(scope.client().cross_mode, 0);
            assert_eq!(scope.client().scene_state, 1);
            panic!("unwind the paint, not the simulation state");
        }));
        assert!(result.is_err());
        assert_eq!(client.scene_state, 2);
        assert_eq!(
            (
                client.cross_mode,
                client.cross_cycle,
                client.cross_x,
                client.cross_y
            ),
            (2, 100, 7, 8),
        );
        client.ingame = false;
        let mut title = LoadingPaintScope::enter(&mut client, true);
        title.cross_paint(Some(test_hooks::RenderTestCross::At { x: 300, y: 180 }));
        assert_eq!(title.client().cross_mode, 2);
        assert_eq!(title.client().scene_state, 2);
    }

    // Observed x86_64 Linux receipts are from drawing-memory-n10-final.json.
    #[cfg(feature = "render-test-hooks")]
    const S_HEAD_CEILING_BYTES: usize = 3_072; // 582 B (23%) above the observed 2,490 B head row.
    #[cfg(feature = "render-test-hooks")]
    const S_GATE_CEILING_BYTES: usize = 64; // 8 B (14%) above the observed 56 B gate row.
    #[cfg(feature = "render-test-hooks")]
    const S_CONSUMER_ACTIVE_CEILING_BYTES: usize = 384; // 48 B (14%) above the observed 336 B active row.
    #[cfg(feature = "render-test-hooks")]
    const S_CONSUMER_DRAINING_CEILING_BYTES: usize = 448; // 56 B (14%) above the observed 392 B draining row.
    #[cfg(feature = "render-test-hooks")]
    const S_SERVICE_CEILING_BYTES: usize = 24_576; // 2,688 B (12%) above the observed 21,888 B process row.

    #[cfg(feature = "render-test-hooks")]
    #[test]
    fn scalar_receipt_tripwire_records_disjoint_owner_rows() {
        let receipt = test_hooks::scalar_receipt();
        assert_eq!(
            receipt.s_head(),
            receipt.head_inline_bytes()
                + receipt.head_thread_local_bytes()
                + receipt.head_wake_arc_allocation_bytes()
                + receipt.head_cell_arc_allocation_bytes()
                + receipt.head_budget_arc_allocation_bytes()
        );
        assert_eq!(
            receipt.s_gate(),
            receipt.present_gate_arc_allocation_bytes()
        );
        assert_eq!(
            receipt.s_consumer(),
            receipt.s_consumer_active_bytes()
                + receipt.present_draining_gate_arc_allocation_bytes()
        );
        assert_eq!(
            receipt.s_service(),
            receipt.render_process_static_bytes()
                + receipt.fair_process_static_bytes()
                + receipt.render_service_arc_allocation_bytes()
                + receipt.process_budget_arc_allocation_bytes()
                + receipt.present_process_service_bytes()
        );
        assert_eq!(
            receipt.mailbox_per_loaded_bot_bytes(),
            FrameBuf::fixed_storage_bytes()
        );
        assert!(
            receipt.s_head() <= S_HEAD_CEILING_BYTES,
            "S_head={} B exceeds {} B ceiling",
            receipt.s_head(),
            S_HEAD_CEILING_BYTES
        );
        assert!(
            receipt.s_gate() <= S_GATE_CEILING_BYTES,
            "S_gate={} B exceeds {} B ceiling",
            receipt.s_gate(),
            S_GATE_CEILING_BYTES
        );
        assert!(
            receipt.s_consumer_active_bytes() <= S_CONSUMER_ACTIVE_CEILING_BYTES,
            "active S_consumer={} B exceeds {} B ceiling",
            receipt.s_consumer_active_bytes(),
            S_CONSUMER_ACTIVE_CEILING_BYTES
        );
        assert!(
            receipt.s_consumer() <= S_CONSUMER_DRAINING_CEILING_BYTES,
            "draining S_consumer={} B exceeds {} B ceiling",
            receipt.s_consumer(),
            S_CONSUMER_DRAINING_CEILING_BYTES
        );
        assert!(
            receipt.s_service() <= S_SERVICE_CEILING_BYTES,
            "S_service={} B exceeds {} B ceiling",
            receipt.s_service(),
            S_SERVICE_CEILING_BYTES
        );
        // Hooks plus draw-memory-profile measure 1,512 B, leaving 24 B below
        // this ceiling. Production has its own no-hooks receipt in slot_io::tests.
        assert!(
            receipt.mailbox_per_loaded_bot_bytes() <= 1_536,
            "instrumented mailbox={} B exceeds 1536 B ceiling",
            receipt.mailbox_per_loaded_bot_bytes(),
        );

        if receipt.process_budget_state_allocation_count() == 0 {
            assert_eq!(receipt.head_budget_state_allocation_count(), 0);
            assert_eq!(receipt.head_budget_arc_allocation_bytes(), 0);
        } else {
            assert_eq!(
                receipt.process_budget_state_allocation_count(),
                client::render::budget::RenderBudget::resource_count()
            );
            assert_eq!(
                receipt.head_budget_state_allocation_count(),
                client::render::budget::RenderBudget::head_local_state_count()
            );
            assert!(receipt.process_budget_arc_allocation_bytes() > 0);
            assert!(receipt.head_budget_arc_allocation_bytes() > 0);
        }
        println!(
            "T4.4 render scalar receipt tripwire: pointer_bytes={} hook_instrumented_mailbox_bytes={} {receipt:?}",
            std::mem::size_of::<usize>(),
            receipt.mailbox_per_loaded_bot_bytes(),
        );
    }
}
