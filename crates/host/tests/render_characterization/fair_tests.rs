//! Feature-only tests over the production slot loop and its real control wake.
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle, ThreadId};

use host::render_head::test_hooks::{RenderTestControl, RenderTestEvent, RenderTestPoint};
use host::render_head::HeadFairStats;

use super::*;

const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug)]
struct Observed {
    event: RenderTestEvent,
    thread: ThreadId,
    at: Instant,
}

#[derive(Clone, Copy, Debug)]
struct Completed {
    stats: Option<HeadFairStats>,
    generation: u64,
    scene_state: i32,
}

struct Gate {
    point: RenderTestPoint,
    release: Receiver<()>,
}

struct Control {
    now: Mutex<Instant>,
    loading: AtomicBool,
    title: AtomicBool,
    draw: AtomicBool,
    stop: AtomicBool,
    resource_failure: Mutex<Option<client::render::backend::gpu::test_hooks::SceneTestPoint>>,
    gate: Mutex<Option<Gate>>,
    events: Sender<Observed>,
    park: Arc<host::SlotPark>,
    self_wake: AtomicBool,
}

impl RenderTestControl for Control {
    fn now(&self) -> Instant {
        *self.now.lock().unwrap()
    }

    fn loading_paint(&self) -> bool {
        self.loading.load(Ordering::SeqCst)
    }

    fn resource_failure(&self) -> Option<client::render::backend::gpu::test_hooks::SceneTestPoint> {
        *self.resource_failure.lock().unwrap()
    }

    fn at(&self, event: RenderTestEvent) {
        if event.point == RenderTestPoint::AfterLeaseRelease {
            self.self_wake
                .store(self.park.wait_readable(Duration::ZERO), Ordering::SeqCst);
        }
        let _ = self.events.send(Observed {
            event,
            thread: thread::current().id(),
            at: self.now(),
        });
        let gate = {
            let mut gate = self.gate.lock().unwrap();
            if gate.as_ref().is_some_and(|gate| gate.point == event.point) {
                gate.take()
            } else {
                None
            }
        };
        if let Some(gate) = gate {
            let _ = gate.release.recv_timeout(TIMEOUT);
        }
    }
}

struct Slot {
    control: Arc<Control>,
    input: Arc<SlotInput>,
    output: Arc<FrameBuf>,
    wake: host::SlotWake,
    events: Receiver<Observed>,
    completed: Receiver<Completed>,
    release: Option<Sender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl Slot {
    fn start(now: Instant, hold: Option<RenderTestPoint>) -> Self {
        let (wake, park) = host::wake_channel();
        let park = Arc::new(park);
        let (events_tx, events) = mpsc::channel();
        let (completed_tx, completed) = mpsc::channel();
        let (release, gate) = hold.map_or((None, None), |point| {
            let (release, receive) = mpsc::channel();
            (
                Some(release),
                Some(Gate {
                    point,
                    release: receive,
                }),
            )
        });
        let control = Arc::new(Control {
            now: Mutex::new(now),
            loading: AtomicBool::new(false),
            title: AtomicBool::new(false),
            draw: AtomicBool::new(true),
            stop: AtomicBool::new(false),
            gate: Mutex::new(gate),
            resource_failure: Mutex::new(None),
            events: events_tx,
            park: Arc::clone(&park),
            self_wake: AtomicBool::new(false),
        });
        let input = SlotInput::new();
        let output = FrameBuf::new();
        let render = SlotRender::new(slot_key(), Arc::clone(&output), Arc::new(wake.clone()))
            .with_test_control(control.clone());
        let worker = {
            let control = Arc::clone(&control);
            let input = Arc::clone(&input);
            let cache = assets();
            let output = Arc::clone(&output);
            thread::spawn(move || {
                let mut client = scene_client(cache.as_deref());
                client.last_response = Some(Instant::now());
                Host::run_client(
                    &mut client,
                    "t4fair",
                    ProfileSettings::default(),
                    Arc::new(AtomicBool::new(false)),
                    Arc::new(AtomicBool::new(false)),
                    Arc::new(Mutex::new(String::new())),
                    Some(input),
                    Some(render),
                    Some(park),
                    ScriptRunPolicy::default(),
                    |client, _, _, _, _| {
                        client.set_draw(control.draw.load(Ordering::SeqCst));
                        client.ingame = !control.title.load(Ordering::SeqCst);
                        false
                    },
                    |client| {
                        let _ = completed_tx.send(Completed {
                            stats: output.fair_stats(),
                            generation: output.generation(),
                            scene_state: client.scene_state,
                        });
                        control.stop.load(Ordering::SeqCst)
                    },
                    |_: &api::DetectedRandom| RandomClaim::Host,
                );
            })
        };
        Self {
            control,
            input,
            output,
            wake,
            events,
            completed,
            release,
            worker: Some(worker),
        }
    }

    fn event(&self, point: RenderTestPoint) -> Observed {
        receive(&self.events, |event| event.event.point == point)
    }

    fn painted(&self, after_generation: u64) -> Completed {
        receive(&self.completed, |completed| {
            completed.generation > after_generation
        })
    }

    fn tick(&self, now: Instant) {
        *self.control.now.lock().unwrap() = now;
        self.wake.wake();
    }

    fn release(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.control.stop.store(true, Ordering::SeqCst);
        self.release();
        self.wake.wake();
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .expect("the production slot exits without panic");
        }
    }
}

#[track_caller]
fn receive<T>(receiver: &Receiver<T>, mut matches: impl FnMut(&T) -> bool) -> T {
    let caller = std::panic::Location::caller();
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let item = receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|error| {
                panic!(
                    "the production slot reaches boundary requested at {}:{}: {error}",
                    caller.file(),
                    caller.line()
                )
            });
        if matches(&item) {
            return item;
        }
    }
}

fn gpu_fixture() -> Option<MutexGuard<'static, ()>> {
    let guard = serial();
    heads().contains(&Head::Gpu).then_some(guard)
}

#[test]
fn peer_slot_release_wakes_parked_watch_head_within_one_tick() {
    let Some(_serial) = gpu_fixture() else {
        return;
    };
    let now = Instant::now();
    let mut owner = Slot::start(now, Some(RenderTestPoint::BeforeLeaseRelease));
    let owner_consumed = owner.event(RenderTestPoint::AfterConsume);
    owner.event(RenderTestPoint::BeforeLeaseRelease);
    let peer = Slot::start(now, None);
    let busy = peer.painted(0);
    assert_eq!(busy.stats.unwrap().scene_consumes(), 0);
    assert_eq!(busy.stats.unwrap().busy_paints(), 1);
    owner.release();
    let consumed = peer.event(RenderTestPoint::AfterConsume);
    let painted = peer.painted(busy.generation);
    assert_ne!(owner_consumed.thread, consumed.thread);
    assert_eq!(
        consumed.at, now,
        "neither watch timeout nor grant expiry advances the controlled clock"
    );
    assert_eq!(painted.stats.unwrap().scene_consumes(), 1);
    assert_eq!(painted.stats.unwrap().due_skips(), 0);
}

#[test]
fn watch_grant_wake_latency_is_measured_from_publication() {
    let Some(_serial) = gpu_fixture() else {
        return;
    };
    let now = Instant::now();
    let mut owner = Slot::start(now, Some(RenderTestPoint::BeforeLeaseRelease));
    owner.event(RenderTestPoint::BeforeLeaseRelease);
    let mut peer = Slot::start(now, Some(RenderTestPoint::OverlayEntry));
    peer.event(RenderTestPoint::OverlayEntry);
    // Keep B inside its first Busy paint until A has published and notified.
    // Its next actual slot tick must therefore start after the notice.
    let published_at = Instant::now();
    *owner.control.now.lock().unwrap() = published_at;
    *peer.control.now.lock().unwrap() = published_at;
    owner.release();
    owner.event(RenderTestPoint::AfterLeaseRelease);
    peer.release();
    let busy = peer.painted(0);
    assert_eq!(busy.stats.unwrap().scene_consumes(), 0);
    let consumed = peer.event(RenderTestPoint::AfterConsume);
    let painted = peer.painted(busy.generation);
    let stats = painted.stats.unwrap();
    let latency = stats
        .last_grant_latency()
        .expect("a real cross-slot wake sample");
    assert_eq!(latency.reservation(), consumed.event.reservation.unwrap());
    assert_eq!(latency.published_at(), published_at);
    assert!(latency.notified_at() >= latency.published_at());
    assert!(latency.woke_at() >= latency.notified_at());
    assert!(latency.consumed_at() >= latency.woke_at());
    assert!(latency.consumed_at() <= Instant::now());
    assert_eq!(stats.grant_wake_samples(), 1);
    assert_eq!(
        stats.max_grant_latency().unwrap().reservation(),
        latency.reservation()
    );
    assert_eq!(stats.scene_releases(), 1);
    assert!(stats.max_scene_lease_time() > Duration::ZERO);
}

#[test]
fn own_scene_publications_do_not_self_wake() {
    let Some(_serial) = gpu_fixture() else {
        return;
    };
    let slot = Slot::start(Instant::now(), None);
    let consumed = slot.event(RenderTestPoint::AfterConsume);
    let released = slot.event(RenderTestPoint::AfterLeaseRelease);
    slot.painted(0);
    assert_eq!(consumed.thread, released.thread);
    assert!(!slot.control.self_wake.load(Ordering::SeqCst));
    assert!(!slot.control.park.wait_readable(Duration::ZERO));
}

#[test]
fn released_head_requests_again_only_on_its_next_due_paint() {
    let Some(_serial) = gpu_fixture() else {
        return;
    };
    let now = Instant::now();
    let slot = Slot::start(now, None);
    let first = slot.painted(0);
    assert_eq!(first.stats.unwrap().scene_consumes(), 1);
    slot.tick(now + Duration::from_millis(20));
    let idle = receive(&slot.completed, |completed| {
        completed.generation == first.generation
    });
    assert_eq!(idle.stats.unwrap().scene_consumes(), 1);
    slot.tick(now + Duration::from_secs(1));
    let second = slot.painted(first.generation);
    assert_eq!(second.stats.unwrap().scene_consumes(), 2);
}

#[test]
fn independent_pump_reports_consumed_overdue_before_owner_returns() {
    let Some(_serial) = gpu_fixture() else {
        return;
    };
    let now = Instant::now();
    let baseline = render_head::workspace_stats().faults().totals();
    let mut owner = Slot::start(now, Some(RenderTestPoint::AfterConsume));
    let consumed = owner.event(RenderTestPoint::AfterConsume);
    assert!(consumed.event.owns_lease);
    let peer = Slot::start(now, None);
    let busy = peer.painted(0);
    let detected = now + Duration::from_millis(40);
    let deadline =
        render_head::pump(detected).expect("a configured owner retains a future heartbeat");
    assert!(deadline.instant() > detected);
    let faults = render_head::workspace_stats().faults();
    assert_eq!(faults.totals()[1], baseline[1] + 1);
    let fault = faults.latest().unwrap();
    assert!(fault.active());
    assert_eq!(fault.reservation().head(), consumed.event.head);
    render_head::acknowledge_fault(fault.sequence()).unwrap();
    assert!(render_head::workspace_stats()
        .faults()
        .latest()
        .unwrap()
        .active());
    for millis in [60, 80, 100] {
        let at = now + Duration::from_millis(millis);
        assert!(render_head::pump(at).unwrap().instant() > at);
    }
    assert_eq!(
        render_head::workspace_stats().faults().totals(),
        faults.totals()
    );
    peer.control.loading.store(true, Ordering::SeqCst);
    peer.tick(now + Duration::from_secs(1));
    let overlay = peer.painted(busy.generation);
    assert_eq!(overlay.stats.unwrap().scene_consumes(), 0);
    assert_eq!(
        overlay.scene_state, 2,
        "the loading scope restores the world state before simulation"
    );
    owner.release();
    owner.event(RenderTestPoint::AfterLeaseRelease);
    assert!(!render_head::workspace_stats()
        .faults()
        .latest()
        .unwrap()
        .active());
}

#[test]
fn late_process_fixture_cannot_reset_live_service() {
    let _serial = serial();
    render_head::pump(Instant::now());
    assert_eq!(
        render_head::test_hooks::configure_process(render_head::test_hooks::ProcessFixture {
            grant_ttl: Duration::from_millis(20),
            quantum: Duration::from_millis(40),
        }),
        Err(host::render::RenderError::ConfigurationLocked)
    );
}

#[test]
fn watch_no_paint_tick_never_self_grants_or_blocks_peer() {
    let Some(_serial) = gpu_fixture() else {
        return;
    };
    let now = Instant::now();
    let watch = Slot::start(now, None);
    let first = watch.painted(0);
    watch.tick(now + Duration::from_millis(20));
    receive(&watch.completed, |completed| {
        completed.generation == first.generation
    });
    let mut peer = Slot::start(
        now + Duration::from_millis(20),
        Some(RenderTestPoint::AfterConsume),
    );
    let consumed = peer.event(RenderTestPoint::AfterConsume);
    assert_eq!(consumed.at, now + Duration::from_millis(20));
    assert_eq!(watch.output.fair_stats().unwrap().scene_consumes(), 1);
    peer.release();
    peer.event(RenderTestPoint::AfterLeaseRelease);
    watch.tick(now + Duration::from_secs(1));
    let second = watch.painted(first.generation);
    assert_eq!(second.stats.unwrap().scene_consumes(), 2);
}

#[test]
fn grant_to_loading_watch_head_cancels_without_paint() {
    let Some(_serial) = gpu_fixture() else {
        return;
    };
    let now = Instant::now();
    let mut owner = Slot::start(now, Some(RenderTestPoint::BeforeLeaseRelease));
    owner.event(RenderTestPoint::BeforeLeaseRelease);
    let peer = Slot::start(now, None);
    let busy = peer.painted(0);
    let busy_pixels = pixels(
        &peer
            .output
            .take()
            .expect("the Busy paint publishes a frame"),
    );
    peer.control.loading.store(true, Ordering::SeqCst);
    owner.release();
    owner.event(RenderTestPoint::AfterLeaseRelease);
    let cancelled = receive(&peer.completed, |completed| {
        completed.generation == busy.generation
    });
    assert_eq!(cancelled.stats.unwrap().scene_consumes(), 0);
    assert_eq!(cancelled.stats.unwrap().oldest_demand_age(), Duration::ZERO);
    let third = Slot::start(now, None);
    assert_eq!(third.painted(0).stats.unwrap().scene_consumes(), 1);
    peer.tick(now + Duration::from_secs(1));
    let loading = peer.painted(busy.generation);
    assert_eq!(loading.stats.unwrap().scene_consumes(), 0);
    assert_eq!(
        loading.stats.unwrap().busy_paints(),
        busy.stats.unwrap().busy_paints()
    );
    assert_eq!(loading.scene_state, 2);
    let loading_pixels = pixels(
        &peer
            .output
            .take()
            .expect("the due loading paint publishes a frame"),
    );
    let (outside_splash, splash) = split_diff(&busy_pixels, &loading_pixels);
    assert_eq!(
        outside_splash, 0,
        "loading preserves the empty retained scene outside its splash"
    );
    if assets().is_some() {
        assert!(
            splash > 0,
            "the due paint publishes the real loading splash pixels"
        );
    }
}

#[test]
fn grant_to_title_watch_head_cancels_without_paint() {
    let Some(_serial) = gpu_fixture() else {
        return;
    };
    let now = Instant::now();
    let mut owner = Slot::start(now, Some(RenderTestPoint::BeforeLeaseRelease));
    owner.event(RenderTestPoint::BeforeLeaseRelease);
    let peer = Slot::start(now, None);
    let busy = peer.painted(0);
    peer.control.title.store(true, Ordering::SeqCst);
    owner.release();
    owner.event(RenderTestPoint::AfterLeaseRelease);
    let cancelled = receive(&peer.completed, |completed| {
        completed.generation == busy.generation
    });
    assert_eq!(cancelled.stats.unwrap().scene_consumes(), 0);
    assert_eq!(cancelled.stats.unwrap().oldest_demand_age(), Duration::ZERO);
    let third = Slot::start(now, None);
    assert_eq!(third.painted(0).stats.unwrap().scene_consumes(), 1);
    peer.tick(now + Duration::from_secs(1));
    let title = peer.painted(busy.generation);
    assert_eq!(title.stats.unwrap().scene_consumes(), 0);
    assert!(!title.stats.unwrap().has_valid_scene());
}

#[test]
fn slot_focus_changes_preserve_oldest_due_order() {
    let Some(_serial) = gpu_fixture() else {
        return;
    };
    for newer_due in [Duration::from_millis(1), Duration::ZERO] {
        let now = Instant::now();
        let mut owner = Slot::start(now, Some(RenderTestPoint::BeforeLeaseRelease));
        owner.event(RenderTestPoint::BeforeLeaseRelease);
        let mut older = Slot::start(now, Some(RenderTestPoint::BeforeLeaseRelease));
        let older_busy = older.painted(0);
        let mut newer = Slot::start(now + newer_due, Some(RenderTestPoint::BeforeLeaseRelease));
        let newer_busy = newer.painted(0);
        newer.input.set_render_focused(true);
        newer.tick(now + Duration::from_millis(2));
        receive(&newer.completed, |completed| {
            completed.generation == newer_busy.generation
        });
        assert!(!older.input.render_focused());
        assert!(
            !newer.input.full_rate(),
            "focus does not depend on the sidecar policy"
        );
        owner.release();
        let (winner, loser, loser_busy) = if newer_due.is_zero() {
            (&mut newer, &mut older, older_busy)
        } else {
            (&mut older, &mut newer, newer_busy)
        };
        let consumed = winner.event(RenderTestPoint::AfterConsume);
        winner.event(RenderTestPoint::BeforeLeaseRelease);
        assert_eq!(consumed.event.head, winner.output.head().unwrap());
        assert_eq!(loser.output.fair_stats().unwrap().scene_consumes(), 0);
        assert_eq!(loser.output.generation(), loser_busy.generation);
        winner.release();
        loser.event(RenderTestPoint::AfterConsume);
        loser.release();
        loser.event(RenderTestPoint::AfterLeaseRelease);
    }
}

#[test]
fn due_skip_counts_opportunities_not_grant_polls() {
    let Some(_serial) = gpu_fixture() else {
        return;
    };
    let now = Instant::now();
    let mut owner = Slot::start(now, Some(RenderTestPoint::BeforeLeaseRelease));
    owner.event(RenderTestPoint::BeforeLeaseRelease);
    let mut peer = Slot::start(now, Some(RenderTestPoint::OverlayEntry));
    peer.event(RenderTestPoint::OverlayEntry);
    owner.release();
    owner.event(RenderTestPoint::AfterLeaseRelease);
    *peer.control.now.lock().unwrap() = now + Duration::from_millis(40);
    render_head::pump(now + Duration::from_millis(40));
    peer.release();
    let busy = peer.painted(0);
    assert_eq!(busy.stats.unwrap().due_skips(), 1);
    for millis in [40, 60, 80] {
        peer.tick(now + Duration::from_millis(millis));
        let polled = receive(&peer.completed, |completed| {
            completed
                .stats
                .is_some_and(|stats| stats.expiry_events() == 1)
        });
        assert_eq!(polled.stats.unwrap().due_skips(), 1);
        assert_eq!(polled.stats.unwrap().expiry_streak(), 1);
    }
    peer.tick(now + Duration::from_secs(1));
    let consumed = peer.painted(busy.generation);
    let stats = consumed.stats.unwrap();
    assert_eq!(stats.scene_consumes(), 1);
    assert_eq!(stats.due_skips(), 0);
    assert_eq!(stats.expiry_streak(), 0);
    assert_eq!(stats.expiry_events(), 1);
    assert_eq!(stats.max_due_skips(), 1);
    peer.control.draw.store(false, Ordering::SeqCst);
    peer.tick(now + Duration::from_millis(1020));
    let retired = receive(&peer.completed, |_| peer.output.head().is_none());
    let final_stats = retired.stats.unwrap();
    assert_eq!(final_stats.identity(), stats.identity());
    assert_eq!(final_stats.max_due_skips(), 1);
    assert_eq!(final_stats.due_skips(), 0);
}

#[test]
fn slot_boundary_hooks_hold_owner_and_report_overlay_release() {
    let Some(_serial) = gpu_fixture() else {
        return;
    };
    let slot = Slot::start(Instant::now(), None);
    let points = [
        RenderTestPoint::AfterConsume,
        RenderTestPoint::BeforeTerminal,
        RenderTestPoint::BeforeLeaseRelease,
        RenderTestPoint::AfterLeaseRelease,
        RenderTestPoint::OverlayEntry,
    ];
    let events = points.map(|point| slot.event(point));
    let owner = events[0].thread;
    let identity = events[0].event.head;
    let reservation = events[0]
        .event
        .reservation
        .expect("the consumed lease has an exact identity");
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event.thread, owner);
        assert_eq!(event.event.head, identity);
        assert_eq!(event.event.owns_lease, index < 3);
        assert_eq!(event.event.reservation, (index < 3).then_some(reservation));
    }
    slot.painted(0);
    assert_eq!(slot.output.head(), Some(identity));
    assert!(!slot.input.render_focused());
}

#[test]
fn lock_fault_survives_failed_paint_and_lease_release() {
    let Some(_serial) = gpu_fixture() else {
        return;
    };
    let now = Instant::now();
    let slot = Slot::start(now, None);
    let first = slot.painted(0);
    let before = render_head::workspace_stats().faults().totals();
    let holder = client::render::backend::gpu::test_hooks::hold_scene_lock();
    slot.tick(now + Duration::from_secs(1));
    let failed = slot.painted(first.generation);
    assert_eq!(failed.stats.unwrap().scene_consumes(), 2);
    assert_eq!(
        failed.stats.unwrap().scene_progress(),
        client::render::backend::SceneProgress::Retained
    );
    let faults = render_head::workspace_stats().faults();
    assert_eq!(faults.totals()[0], before[0] + 1);
    let incident = faults.latest().unwrap();
    assert_eq!(
        incident.kind(),
        host::render::RenderFaultKind::LockContention
    );
    assert!(!incident.active());
    assert!(!slot.control.self_wake.load(Ordering::SeqCst));
    drop(holder);
    render_head::acknowledge_fault(incident.sequence()).unwrap();
    let acknowledged = render_head::workspace_stats().faults();
    assert_eq!(acknowledged.totals(), faults.totals());
    assert_eq!(acknowledged.acknowledged_sequence(), incident.sequence());
}

#[test]
fn resource_failure_releases_before_overlay_and_retains_the_scene() {
    let Some(_serial) = gpu_fixture() else {
        return;
    };
    let now = Instant::now();
    let slot = Slot::start(now, None);
    let first = slot.painted(0);
    slot.event(RenderTestPoint::OverlayEntry);
    let before = render_head::workspace_stats().faults().totals();
    *slot.control.resource_failure.lock().unwrap() =
        Some(client::render::backend::gpu::test_hooks::SceneTestPoint::BeforeTerminal);
    slot.tick(now + Duration::from_secs(1));
    assert!(
        slot.event(RenderTestPoint::BeforeLeaseRelease)
            .event
            .owns_lease
    );
    assert!(
        !slot
            .event(RenderTestPoint::AfterLeaseRelease)
            .event
            .owns_lease
    );
    assert!(!slot.event(RenderTestPoint::OverlayEntry).event.owns_lease);
    let failed = slot.painted(first.generation);
    assert!(failed.stats.unwrap().has_valid_scene());
    assert_eq!(
        failed.stats.unwrap().scene_progress(),
        client::render::backend::SceneProgress::Retained
    );
    let faults = render_head::workspace_stats().faults();
    assert_eq!(faults.totals()[2], before[2] + 1);
    assert!(!faults.latest().unwrap().active());
}
