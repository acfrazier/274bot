//! Feature-only fair-render activation. Production keeps the wrap until S5.
use std::cell::Cell;
use std::marker::PhantomData;
use std::num::NonZeroU64;
use std::rc::Rc;
use std::sync::OnceLock;

use client::render::backend::gpu::{SceneAdmission, SceneSubmit};
use client::render::backend::SceneProgress;

use super::*;
use crate::render::ReservationState;

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

    pub trait RenderTestControl: Send + Sync {
        fn now(&self) -> Instant;
        fn loading_paint(&self) -> bool {
            false
        }
        fn resource_failure(
            &self,
        ) -> Option<client::render::backend::gpu::test_hooks::SceneTestPoint> {
            None
        }
        fn at(&self, event: RenderTestEvent);
    }

    pub fn configure_process(fixture: ProcessFixture) -> Result<(), RenderError> {
        let mut config = lock(&CONFIG);
        if config.started || config.fixture.is_some() {
            return Err(RenderError::ConfigurationLocked);
        }
        // Validate before storing the fixture, without initializing PROCESS.
        let service = RenderService::new(fixture.grant_ttl)?;
        service.set_consumed_quantum(fixture.quantum)?;
        config.fixture = Some(fixture);
        Ok(())
    }
}

struct Configuration {
    started: bool,
    fixture: Option<test_hooks::ProcessFixture>,
}
static CONFIG: Mutex<Configuration> = Mutex::new(Configuration {
    started: false,
    fixture: None,
});
static FIXTURE: OnceLock<Option<test_hooks::ProcessFixture>> = OnceLock::new();

pub(super) fn initialize() -> Option<test_hooks::ProcessFixture> {
    *FIXTURE.get_or_init(|| {
        let mut config = lock(&CONFIG);
        config.started = true;
        config.fixture
    })
}

pub(super) fn enabled() -> bool {
    FIXTURE.get().is_some_and(Option::is_some)
}

pub(super) fn heartbeat(now: Instant, deadline: Option<Deadline>) -> Option<Deadline> {
    let fixture = FIXTURE.get().copied().flatten()?;
    let beat = Deadline::at(
        now.checked_add(fixture.quantum)
            .expect("validated fixture quantum"),
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

/// A fixed-size identity-checked sample. No renderer container traversal occurs here.
#[derive(Debug, Clone, Copy)]
pub struct HeadFairStats {
    identity: HeadIdentity,
    due_skips: u64,
    max_due_skips: u64,
    expiry_streak: u64,
    max_expiry_streak: u64,
    expiry_events: u64,
    busy_paints: u64,
    busy_paint_time: Duration,
    granted_paints: u64,
    granted_paint_time: Duration,
    scene_consumes: u64,
    last_opportunity: Option<DueOpportunity>,
    last_expired: Option<ReservationIdentity>,
    oldest_demand_age: Duration,
    backend: BackendKind,
    has_valid_scene: bool,
    scene_progress: SceneProgress,
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
    control: Option<Arc<dyn test_hooks::RenderTestControl>>,
    opportunity_exhausted: bool,
}

impl FairHead {
    pub(super) fn new(
        identity: HeadIdentity,
        backend: BackendKind,
        control: Option<Arc<dyn test_hooks::RenderTestControl>>,
    ) -> Self {
        Self {
            demand: SceneDemand::default(),
            stats: HeadFairStats {
                identity,
                due_skips: 0,
                max_due_skips: 0,
                expiry_streak: 0,
                max_expiry_streak: 0,
                expiry_events: 0,
                busy_paints: 0,
                busy_paint_time: Duration::ZERO,
                granted_paints: 0,
                granted_paint_time: Duration::ZERO,
                scene_consumes: 0,
                last_opportunity: None,
                last_expired: None,
                oldest_demand_age: Duration::ZERO,
                backend,
                has_valid_scene: false,
                scene_progress: SceneProgress::NoScene,
            },
            next_opportunity: NonZeroU64::new(1),
            opportunity: None,
            focused: None,
            control,
            opportunity_exhausted: false,
        }
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
            report(PROCESS.service.set_focused(lease, focused, now));
            self.fair.focused = Some(focused);
        }
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
    static OWNS_LEASE: Cell<bool> = const { Cell::new(false) };
    static RESERVATION: Cell<Option<ReservationIdentity>> = const { Cell::new(None) };
    static CONTROL: RefCell<Option<(HeadIdentity, Arc<dyn test_hooks::RenderTestControl>)>> = const { RefCell::new(None) };
}

struct HostPaintScope {
    previous: bool,
    previous_control: Option<(HeadIdentity, Arc<dyn test_hooks::RenderTestControl>)>,
    previous_lease: bool,
    previous_reservation: Option<ReservationIdentity>,
    _thread_bound: PhantomData<Rc<()>>,
}
impl HostPaintScope {
    fn enter(head: &Head) -> Self {
        Self {
            previous: HOST_ACTIVE.replace(true),
            previous_control: CONTROL.replace(
                head.fair
                    .control
                    .as_ref()
                    .map(|control| (head.identity, Arc::clone(control))),
            ),
            previous_lease: OWNS_LEASE.replace(false),
            previous_reservation: RESERVATION.replace(None),
            _thread_bound: PhantomData,
        }
    }
}
impl Drop for HostPaintScope {
    fn drop(&mut self) {
        HOST_ACTIVE.set(self.previous);
        CONTROL.replace(self.previous_control.take());
        OWNS_LEASE.set(self.previous_lease);
        RESERVATION.set(self.previous_reservation);
    }
}

pub(super) fn host_active() -> bool {
    HOST_ACTIVE.get()
}

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

fn bridge(point: client::render::backend::gpu::test_hooks::SceneTestPoint) {
    use client::render::backend::gpu::test_hooks::SceneTestPoint;
    match point {
        SceneTestPoint::AfterAdmission => {}
        SceneTestPoint::BeforeTerminal => event(test_hooks::RenderTestPoint::BeforeTerminal),
        SceneTestPoint::OverlayEntry => event(test_hooks::RenderTestPoint::OverlayEntry),
    }
}

pub(super) struct LoadingPaintScope<'a> {
    client: &'a mut Client,
    previous: Option<i32>,
}
impl<'a> LoadingPaintScope<'a> {
    pub(super) fn enter(client: &'a mut Client, loading: bool) -> Self {
        let previous = (loading && client.ingame).then_some(client.scene_state);
        if previous.is_some() {
            client.scene_state = 1;
        }
        Self { client, previous }
    }
    fn client(&mut self) -> &mut Client {
        self.client
    }
}
impl Drop for LoadingPaintScope<'_> {
    fn drop(&mut self) {
        if let Some(previous) = self.previous {
            self.client.scene_state = previous;
        }
    }
}

pub(super) struct FairFrame {
    head: FairHead,
    lease: Option<FrameLease>,
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
        let empty = FairHead::new(state.stats.identity, state.stats.backend, None);
        let lent_state = std::mem::replace(state, empty);
        let frame = GpuFrame {
            lease: lease.take().expect("attached head owns its lease"),
            wake: Arc::clone(wake),
            scenes: *scenes,
            fair: Some(FairFrame {
                head: lent_state,
                lease: None,
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
    let _observer = client::render::backend::gpu::test_hooks::install_for_thread(bridge);
    let failure = head
        .fair
        .control
        .as_ref()
        .and_then(|control| control.resource_failure());
    let _failure = failure.map(client::render::backend::gpu::test_hooks::resource_failure_at);
    let loading = head
        .fair
        .control
        .as_ref()
        .is_some_and(|control| control.loading_paint());
    let mut loading_scope = LoadingPaintScope::enter(client, loading);
    let start = Instant::now();
    let consumes = head.fair.stats.scene_consumes;
    let output = if head.backend_kind() == BackendKind::Gpu {
        let _lent = FairLent::lend(
            &mut head.lease,
            &mut head.scenes,
            &mut head.fair,
            &head.wake,
        );
        head.renderer.mainredraw(loading_scope.client())
    } else {
        head.renderer.mainredraw(loading_scope.client())
    };
    let elapsed = start.elapsed();
    if head.backend_kind() == BackendKind::Gpu {
        if head.fair.stats.scene_consumes > consumes {
            head.fair.stats.granted_paints = head.fair.stats.granted_paints.saturating_add(1);
            head.fair.stats.granted_paint_time =
                head.fair.stats.granted_paint_time.saturating_add(elapsed);
        } else if loading_scope.client().scene_state == 2 {
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
                }
            }
        }
    }
    head.fair.stats.has_valid_scene = head.renderer.has_valid_scene();
    head.fair.stats.scene_progress = head.renderer.scene_progress();
    head.output.publish_fair_stats(head.fair.stats);
    output
}

pub(super) fn admission() -> SceneAdmission {
    GPU_FRAME.with(|cell| {
        let mut frame = cell.borrow_mut();
        let Some(frame) = frame.as_mut() else {
            return SceneAdmission::Standalone;
        };
        let now = frame
            .fair
            .as_ref()
            .and_then(|fair| fair.head.control.as_ref())
            .map_or_else(Instant::now, |control| control.now());
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
    fair.head.refresh(frame.lease.snapshot().1, now);
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
            fair.lease = Some(lease);
            fair.head.demand = SceneDemand::default();
            fair.head.stats.due_skips = 0;
            fair.head.stats.expiry_streak = 0;
            fair.head.stats.scene_consumes = fair.head.stats.scene_consumes.saturating_add(1);
            fair.head.stats.oldest_demand_age = Duration::ZERO;
            OWNS_LEASE.set(true);
            RESERVATION.set(Some(identity));
            event(test_hooks::RenderTestPoint::AfterConsume);
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
    let now = fair
        .head
        .control
        .as_ref()
        .map_or_else(Instant::now, |control| control.now());
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
    event(test_hooks::RenderTestPoint::BeforeLeaseRelease);
    report(lease.finish(now));
    OWNS_LEASE.set(false);
    RESERVATION.set(None);
    event(test_hooks::RenderTestPoint::AfterLeaseRelease);
}

fn report(result: Result<(), RenderError>) {
    if let Err(error) = result {
        host_log!(Category::Lifecycle, Level::Warn, "fair render: {error}");
    }
}

#[cfg(test)]
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
                head: FairHead::new(identity, BackendKind::Gpu, None),
                lease: None,
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
    fn fair_stats_follow_head_generation_not_username() {
        let (service, pump, mut old) = local_heads();
        let output = FrameBuf::new();
        let mut previous = FairHead::new(old.identity(), BackendKind::Gpu, None).stats;
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
        let fresh = FairHead::new(current.identity(), BackendKind::Gpu, None).stats;
        output.publish_fair_stats(fresh);
        assert_eq!(output.fair_stats().unwrap().identity(), current.identity());
        assert_eq!(output.fair_stats().unwrap().max_due_skips(), 0);
        output.publish_fair_stats(previous);
        assert_eq!(output.fair_stats().unwrap().identity(), current.identity());
    }

    #[test]
    fn due_opportunity_exhaustion_never_wraps_or_reuses_an_identity() {
        let (_, _, lease) = local_heads();
        let mut head = FairHead::new(lease.identity(), BackendKind::Gpu, None);
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
    fn loading_paint_scope_restores_client_state_after_render_and_unwind() {
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
        {
            let mut scope = LoadingPaintScope::enter(&mut client, true);
            assert_eq!(scope.client().scene_state, 1);
        }
        assert_eq!(client.scene_state, 2);
        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut scope = LoadingPaintScope::enter(&mut client, true);
            assert_eq!(scope.client().scene_state, 1);
            panic!("unwind the paint, not the simulation state");
        }));
        assert!(result.is_err());
        assert_eq!(client.scene_state, 2);
        client.ingame = false;
        let mut title = LoadingPaintScope::enter(&mut client, true);
        assert_eq!(title.client().scene_state, 2);
    }
}
