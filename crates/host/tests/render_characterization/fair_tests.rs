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
    tick: u64,
}

struct Gate {
    point: RenderTestPoint,
    release: Receiver<()>,
    hold_assets: bool,
}

struct Control {
    now: Mutex<Instant>,
    loading: AtomicBool,
    title: AtomicBool,
    scene_state: Mutex<i32>,
    draw: AtomicBool,
    stop: AtomicBool,
    erase_world: AtomicBool,
    npc_position: Mutex<Option<(i32, i32)>>,
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
        let observed = Observed {
            event,
            thread: thread::current().id(),
            at: self.now(),
        };
        let gate = {
            let mut gate = self.gate.lock().unwrap();
            if gate.as_ref().is_some_and(|gate| gate.point == event.point) {
                gate.take()
            } else {
                None
            }
        };
        match gate {
            Some(gate) if gate.hold_assets => {
                let result =
                    client::render::backend::gpu::test_hooks::with_shared_assets_locked(|| {
                        let _ = self.events.send(observed);
                        let _ = gate.release.recv_timeout(TIMEOUT);
                    });
                result.expect("the GPU slot holds its process asset mutex at the boundary");
            }
            Some(gate) => {
                let _ = self.events.send(observed);
                let _ = gate.release.recv_timeout(TIMEOUT);
            }
            None => {
                let _ = self.events.send(observed);
            }
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
        Self::start_with(now, hold, false, false, false, false, None)
    }

    fn start_with(
        now: Instant,
        hold: Option<RenderTestPoint>,
        hold_assets: bool,
        loading: bool,
        focused: bool,
        full_rate: bool,
        npc_position: Option<(i32, i32)>,
    ) -> Self {
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
                    hold_assets,
                }),
            )
        });
        let control = Arc::new(Control {
            now: Mutex::new(now),
            loading: AtomicBool::new(loading),
            title: AtomicBool::new(false),
            scene_state: Mutex::new(2),
            draw: AtomicBool::new(true),
            stop: AtomicBool::new(false),
            erase_world: AtomicBool::new(false),
            npc_position: Mutex::new(npc_position),
            gate: Mutex::new(gate),
            resource_failure: Mutex::new(None),
            events: events_tx,
            park: Arc::clone(&park),
            self_wake: AtomicBool::new(false),
        });
        let input = SlotInput::new();
        input.set_render_focused(focused);
        input.set_full_rate(full_rate);
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
                if let Some((x, z)) = *control.npc_position.lock().unwrap() {
                    health_bar_npc(&mut client, x, z);
                }
                client.last_response = Some(Instant::now());
                let mut completed_ticks = 0u64;
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
                        if control.erase_world.load(Ordering::SeqCst) {
                            client.world.reset_map();
                        }
                        if let Some((x, z)) = *control.npc_position.lock().unwrap() {
                            health_bar_npc(client, x, z);
                        }
                        *control.scene_state.lock().unwrap() = client.scene_state;
                        false
                    },
                    |client| {
                        completed_ticks = completed_ticks.wrapping_add(1);
                        let _ = completed_tx.send(Completed {
                            stats: output.fair_stats(),
                            generation: output.generation(),
                            scene_state: client.scene_state,
                            tick: completed_ticks,
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

    #[track_caller]
    fn event(&self, point: RenderTestPoint) -> Observed {
        let caller = std::panic::Location::caller();
        let deadline = Instant::now() + TIMEOUT;
        let mut observed = Vec::new();
        loop {
            let item = self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| {
                    let scene_state = *self.control.scene_state.lock().unwrap();
                    let controlled_now = *self.control.now.lock().unwrap();
                    panic!(
                        "the production slot reaches boundary requested at {}:{}: {error}; observed={observed:?}; generation={}; fair_stats={:?}; refusal={:?}; scene_state={}; controlled_now={:?}; draw={}; title={}",
                        caller.file(),
                        caller.line(),
                        self.output.generation(),
                        self.output.fair_stats(),
                        self.output.refusal(),
                        scene_state,
                        controlled_now,
                        self.control.draw.load(Ordering::SeqCst),
                        self.control.title.load(Ordering::SeqCst),
                    )
                });
            observed.push(item);
            if item.event.point == point {
                return item;
            }
        }
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

    fn hold_next_at(&mut self, point: RenderTestPoint, hold_assets: bool) {
        let (release, receive) = mpsc::channel();
        let mut gate = self.control.gate.lock().unwrap();
        assert!(gate.is_none(), "only one boundary gate may be outstanding");
        *gate = Some(Gate {
            point,
            release: receive,
            hold_assets,
        });
        self.release = Some(release);
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

fn gpu_available() -> bool {
    let available = heads().contains(&Head::Gpu);
    assert!(
        !required("T4_REQUIRE_GPU") || available,
        "T4_REQUIRE_GPU=1 but this fixture selected no GPU head"
    );
    available
}

fn gpu_fixture() -> Option<MutexGuard<'static, ()>> {
    let guard = serial();
    if !gpu_available() {
        return None;
    }
    // The first slot tick calls render_now before Head::attach initializes
    // PROCESS. Initialize the configured fair service first, so due_at and
    // the first admission request both use RenderTestControl::now rather than
    // mixing wall-clock demand with the controlled service clock.
    render_head::pump(Instant::now())
        .expect("the configured fair pump is live before the first slot tick");
    Some(guard)
}

fn isolate_test_child(test_name: &str, marker: &str) -> bool {
    if std::env::var_os(marker).is_some() {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test_name, "--nocapture"])
        .env(marker, "1")
        .output()
        .expect("spawn a fresh render-test-hooks process");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("running 1 test"),
        "isolated test child did not complete its one expected test ({:?}):\n{}\n{}",
        output.status,
        stdout,
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

fn gpu_budget_fixture() -> Option<MutexGuard<'static, ()>> {
    let guard = serial();
    let unlimited = usize::MAX;
    render_head::test_hooks::configure_render_budgets(
        client::render::budget::RenderBudgetLimits::new(
            unlimited, unlimited, unlimited, unlimited, unlimited, unlimited, unlimited, unlimited,
            unlimited, unlimited, unlimited, unlimited, unlimited,
        ),
    )
    .expect("the isolated GPU fixture configures its explicit render budgets before PROCESS");
    render_head::pump(Instant::now())
        .expect("the configured fair pump is live before the first slot tick");
    // A bare GpuBackend probe would initialize the process context outside
    // its budget, and a later real head must reject that uncharged context.
    let probe = render_head::test_hooks::CaptureFixture::new(slot_key(), false, false)
        .expect("the GPU probe attaches through the real process budget scope");
    let available = probe.backend_kind() == BackendKind::Gpu;
    assert!(
        !required("T4_REQUIRE_GPU") || available,
        "T4_REQUIRE_GPU=1 but the budget-scoped fixture selected no GPU head"
    );
    available.then_some(guard)
}

#[test]
fn busy_paint_returns_before_peer_release() {
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
fn peer_slot_release_wakes_parked_watch_head_within_one_tick() {
    let Some(_serial) = gpu_fixture() else {
        return;
    };
    let now = Instant::now();
    let mut owner = Slot::start(now, Some(RenderTestPoint::BeforeLeaseRelease));
    owner.event(RenderTestPoint::BeforeLeaseRelease);
    let peer = Slot::start(now, None);
    let busy = peer.painted(0);
    assert_eq!(busy.stats.unwrap().scene_consumes(), 0);
    assert_eq!(busy.stats.unwrap().busy_paints(), 1);
    assert!(
        !peer.control.park.wait_readable(Duration::from_millis(20)),
        "the watch slot has no pending control wake before its peer releases"
    );
    assert!(
        matches!(peer.completed.try_recv(), Err(mpsc::TryRecvError::Empty)),
        "the watch slot remains parked rather than taking an eager tick"
    );

    owner.release();
    let consumed = peer.event(RenderTestPoint::AfterConsume);
    let painted = peer.painted(busy.generation);
    assert_eq!(
        painted.tick,
        busy.tick + 1,
        "the next production slot tick consumes"
    );
    assert_eq!(
        consumed.at, now,
        "the controlled watch deadline never advances"
    );
    let stats = painted.stats.unwrap();
    assert_eq!(stats.scene_consumes(), 1);
    assert_eq!(stats.due_skips(), 0);
    let wake = stats
        .last_grant_latency()
        .expect("the production control wake reaches the target slot");
    assert_eq!(wake.reservation(), consumed.event.reservation.unwrap());
    assert_eq!(wake.published_at(), now);
    assert!(wake.woke_at() >= wake.notified_at());
    assert!(wake.consumed_at() >= wake.woke_at());
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
fn client_scene_lock_holder_blocks_only_the_try_lock_attempt() {
    let Some(_serial) = gpu_fixture() else {
        return;
    };
    let now = Instant::now();
    let slot = Slot::start(now, None);
    let first = slot.painted(0);
    slot.event(RenderTestPoint::OverlayEntry);
    let before = render_head::workspace_stats().faults().totals();
    let holder = client::render::backend::gpu::test_hooks::hold_scene_lock();
    slot.tick(now + Duration::from_secs(1));
    assert!(slot.event(RenderTestPoint::AfterConsume).event.owns_lease);
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
    let failed = receive(&slot.completed, |completed| {
        completed
            .stats
            .is_some_and(|stats| stats.scene_consumes() == 2)
    });
    assert_eq!(failed.generation, first.generation);
    assert!(matches!(
        slot.output.refusal(),
        Some(client::render::backend::RenderFault::Budget(_))
    ));
    assert!(failed.stats.unwrap().has_valid_scene());
    assert_eq!(
        failed.stats.unwrap().scene_progress(),
        client::render::backend::SceneProgress::Retained
    );
    let faults = render_head::workspace_stats().faults();
    assert_eq!(faults.totals()[2], before[2] + 1);
    assert!(!faults.latest().unwrap().active());
}

#[test]
fn busy_skips_scene_mutation_and_keeps_current_overlays() {
    let Some(_serial) = gpu_fixture() else {
        return;
    };
    let now = Instant::now();
    let faults_before = render_head::workspace_stats().faults().totals();
    let mut peer = Slot::start(now, None);
    let first = peer.painted(0);
    peer.event(RenderTestPoint::OverlayEntry);
    let baseline = pixels(&peer.output.take().expect("the baseline frame is published"));

    let mut owner = Slot::start(now, Some(RenderTestPoint::AfterConsume));
    let owner_consumed = owner.event(RenderTestPoint::AfterConsume);
    assert!(owner_consumed.event.owns_lease);
    peer.hold_next_at(RenderTestPoint::OverlayEntry, false);
    peer.control.erase_world.store(true, Ordering::SeqCst);
    *peer.control.npc_position.lock().unwrap() = Some((16 * 128, 12 * 128));
    peer.tick(now + Duration::from_secs(1));

    let overlay = peer.event(RenderTestPoint::OverlayEntry);
    assert!(!overlay.event.owns_lease);
    peer.release();
    let busy = peer.painted(first.generation);
    let stats = busy.stats.unwrap();
    assert_eq!(stats.scene_consumes(), 1);
    assert!(stats.has_valid_scene());
    assert_eq!(
        stats.scene_progress(),
        client::render::backend::SceneProgress::Retained
    );
    let current = pixels(&peer.output.take().expect("the busy frame is published"));
    assert_one_health_bar(
        Head::Gpu,
        "busy paint",
        &overlay_change(&baseline, &current),
        BAR_AHEAD,
    );

    *owner.control.now.lock().unwrap() = now + Duration::from_secs(1);
    owner.release();
    owner.event(RenderTestPoint::AfterLeaseRelease);
    peer.event(RenderTestPoint::AfterConsume);
    let faults = render_head::workspace_stats().faults();
    assert_eq!(faults.totals()[1], faults_before[1] + 1);
    let incident = faults.latest().unwrap();
    assert_eq!(
        incident.reservation(),
        owner_consumed.event.reservation.unwrap()
    );
    assert!(!incident.active());
    render_head::acknowledge_fault(incident.sequence()).unwrap();
    peer.painted(busy.generation);
}

#[test]
fn pump_expires_unconsumed_grant_then_next_paint_can_consume() {
    let Some(_serial) = gpu_fixture() else {
        return;
    };
    let now = Instant::now();
    let mut owner = Slot::start(now, Some(RenderTestPoint::AfterConsume));
    assert!(owner.event(RenderTestPoint::AfterConsume).event.owns_lease);
    let mut peer = Slot::start(now, Some(RenderTestPoint::OverlayEntry));
    assert!(!peer.event(RenderTestPoint::OverlayEntry).event.owns_lease);

    owner.release();
    owner.event(RenderTestPoint::AfterLeaseRelease);
    let expired_at = now + Duration::from_millis(21);
    render_head::pump(expired_at).expect("the independent pump expires the unconsumed grant");
    peer.tick(expired_at);
    peer.release();

    let busy = peer.painted(0);
    assert_eq!(busy.stats.unwrap().scene_consumes(), 0);
    let consumed = peer.event(RenderTestPoint::AfterConsume);
    assert_eq!(consumed.at, expired_at);
    let rebuilt = peer.painted(busy.generation);
    assert_eq!(rebuilt.stats.unwrap().scene_consumes(), 1);
    assert_eq!(rebuilt.stats.unwrap().due_skips(), 0);
}

#[test]
fn rebuild_consumes_exactly_once_and_releases_before_overlays() {
    let Some(_serial) = gpu_fixture() else {
        return;
    };
    let mut slot = Slot::start(Instant::now(), Some(RenderTestPoint::OverlayEntry));
    let consumed = slot.event(RenderTestPoint::AfterConsume);
    assert!(consumed.event.owns_lease);
    let before_release = slot.event(RenderTestPoint::BeforeLeaseRelease);
    assert!(before_release.event.owns_lease);
    assert_eq!(before_release.event.reservation, consumed.event.reservation);
    let after_release = slot.event(RenderTestPoint::AfterLeaseRelease);
    assert!(!after_release.event.owns_lease);
    assert_eq!(after_release.event.reservation, None);
    let overlay = slot.event(RenderTestPoint::OverlayEntry);
    assert!(!overlay.event.owns_lease);
    assert_eq!(overlay.event.reservation, None);
    slot.release();
    let completed = slot.painted(0);
    let stats = completed.stats.unwrap();
    assert_eq!(stats.scene_consumes(), 1);
    assert_eq!(stats.scene_releases(), 1);
    assert!(stats.has_valid_scene());
    assert_eq!(
        stats.scene_progress(),
        client::render::backend::SceneProgress::Rebuilt
    );
    assert_eq!(slot.output.head(), Some(consumed.event.head));
}

#[test]
fn equal_due_game_pane_head_wins_with_sidecar_full_rate() {
    let Some(_serial) = gpu_fixture() else {
        return;
    };
    let now = Instant::now();
    let mut owner = Slot::start(now, Some(RenderTestPoint::BeforeLeaseRelease));
    owner.event(RenderTestPoint::BeforeLeaseRelease);

    let sidecar = Slot::start_with(now, None, false, false, false, true, None);
    let sidecar_busy = sidecar.painted(0);
    sidecar.event(RenderTestPoint::OverlayEntry);
    assert_eq!(sidecar_busy.stats.unwrap().scene_consumes(), 0);

    let mut game = Slot::start_with(
        now,
        Some(RenderTestPoint::AfterConsume),
        false,
        false,
        true,
        false,
        None,
    );
    let game_busy = game.painted(0);
    assert_eq!(game_busy.stats.unwrap().scene_consumes(), 0);
    game.event(RenderTestPoint::OverlayEntry);
    assert!(game.input.render_focused());
    assert!(!sidecar.input.render_focused());

    owner.release();
    owner.event(RenderTestPoint::AfterLeaseRelease);
    let consumed = game.event(RenderTestPoint::AfterConsume);
    assert!(consumed.event.owns_lease);
    assert_eq!(consumed.at, now);
    assert_eq!(sidecar.output.fair_stats().unwrap().scene_consumes(), 0);
    game.release();
    let before_release = game.event(RenderTestPoint::BeforeLeaseRelease);
    assert_eq!(before_release.event.reservation, consumed.event.reservation);

    assert!(
        !game
            .event(RenderTestPoint::AfterLeaseRelease)
            .event
            .owns_lease
    );
    assert!(!game.event(RenderTestPoint::OverlayEntry).event.owns_lease);
    let painted = game.painted(game_busy.generation);
    assert_eq!(painted.stats.unwrap().scene_consumes(), 1);
    let sidecar_consumed = sidecar.event(RenderTestPoint::AfterConsume);
    assert!(sidecar_consumed.event.owns_lease);
    assert_eq!(sidecar_consumed.event.head, sidecar.output.head().unwrap());
    sidecar.event(RenderTestPoint::BeforeLeaseRelease);
    sidecar.event(RenderTestPoint::AfterLeaseRelease);
    sidecar.event(RenderTestPoint::OverlayEntry);
    let sidecar_painted = receive(&sidecar.completed, |completed| {
        completed.generation > sidecar_busy.generation
            && completed
                .stats
                .is_some_and(|stats| stats.scene_consumes() == 1)
    });
    assert_eq!(sidecar_painted.stats.unwrap().scene_consumes(), 1);
}

#[test]
fn host_scene_capture_capacity_is_process_owned() {
    if isolate_test_child(
        "fair_tests::host_scene_capture_capacity_is_process_owned",
        "T44_HOST_SCENE_CAPTURE_CHILD",
    ) {
        return;
    }
    let Some(_serial) = gpu_budget_fixture() else {
        return;
    };
    let now = Instant::now();
    let first = Slot::start(now, None);
    let first_frame = first.painted(0);
    assert_eq!(first_frame.stats.unwrap().scene_consumes(), 1);
    let first_capacity = client::render::backend::gpu::process_capture_storage_bytes();
    assert!(
        first_capacity > 0,
        "the first real scene retains capture storage"
    );

    let second = Slot::start(now, None);
    let second_frame = second.painted(0);
    assert_eq!(second_frame.stats.unwrap().scene_consumes(), 1);
    assert_ne!(first.output.head(), second.output.head());
    assert_eq!(
        client::render::backend::gpu::process_capture_storage_bytes(),
        first_capacity,
        "a second attached GPU head reuses the one process capture workspace"
    );
}

#[test]
fn capture_guard_is_free_before_released_peer_can_consume() {
    if isolate_test_child(
        "fair_tests::capture_guard_is_free_before_released_peer_can_consume",
        "T44_CAPTURE_GUARD_CHILD",
    ) {
        return;
    }
    let Some(_serial) = gpu_budget_fixture() else {
        return;
    };
    let now = Instant::now();
    let before = render_head::workspace_stats().faults().totals();
    let mut owner = Slot::start(now, Some(RenderTestPoint::OverlayEntry));
    let released = owner.event(RenderTestPoint::AfterLeaseRelease);
    assert!(!released.event.owns_lease);
    let overlay = owner.event(RenderTestPoint::OverlayEntry);
    assert!(!overlay.event.owns_lease);

    let mut peer = Slot::start(now, Some(RenderTestPoint::OverlayEntry));
    let consumed = peer.event(RenderTestPoint::AfterConsume);
    assert!(consumed.event.owns_lease);
    assert!(
        peer.event(RenderTestPoint::BeforeLeaseRelease)
            .event
            .owns_lease
    );
    assert!(
        !peer
            .event(RenderTestPoint::AfterLeaseRelease)
            .event
            .owns_lease
    );
    assert!(!peer.event(RenderTestPoint::OverlayEntry).event.owns_lease);
    peer.release();
    let completed = peer.painted(0);
    let stats = completed.stats.unwrap();
    assert_eq!(stats.scene_consumes(), 1);
    assert!(stats.has_valid_scene());
    assert_eq!(
        stats.scene_progress(),
        client::render::backend::SceneProgress::Rebuilt
    );
    assert_eq!(
        render_head::workspace_stats().faults().totals(),
        before,
        "the peer capture succeeds while the first callback remains paused"
    );

    owner.release();
    owner.painted(0);
}

#[test]
fn attach_clones_model_binding_while_peer_holds_assets() {
    if isolate_test_child(
        "fair_tests::attach_clones_model_binding_while_peer_holds_assets",
        "T44_ATTACH_ASSETS_CHILD",
    ) {
        return;
    }
    let Some(_serial) = gpu_budget_fixture() else {
        return;
    };
    let now = Instant::now();
    let mut owner = Slot::start_with(
        now,
        Some(RenderTestPoint::OverlayEntry),
        true,
        false,
        false,
        false,
        None,
    );
    owner.event(RenderTestPoint::OverlayEntry);

    let peer = Slot::start_with(now, None, false, true, false, false, None);
    let attached = peer.painted(0);
    assert_eq!(attached.stats.unwrap().scene_consumes(), 0);
    assert!(peer.output.head().is_some());
    assert_eq!(peer.output.refusal(), None);

    owner.release();
    owner.painted(0);
}

#[test]
fn host_asset_contention_releases_before_overlays() {
    if isolate_test_child(
        "fair_tests::host_asset_contention_releases_before_overlays",
        "T44_HOST_ASSET_CONTENTION_CHILD",
    ) {
        return;
    }
    let Some(_serial) = gpu_budget_fixture() else {
        return;
    };
    let now = Instant::now();
    let peer = Slot::start(now, None);
    let first = peer.painted(0);
    peer.event(RenderTestPoint::OverlayEntry);
    let before = render_head::workspace_stats().faults().totals();

    let mut owner = Slot::start_with(
        now,
        Some(RenderTestPoint::OverlayEntry),
        true,
        false,
        false,
        false,
        None,
    );
    owner.event(RenderTestPoint::OverlayEntry);
    peer.tick(now + Duration::from_secs(1));
    let consumed = peer.event(RenderTestPoint::AfterConsume);
    assert!(consumed.event.owns_lease);
    assert!(
        peer.event(RenderTestPoint::BeforeLeaseRelease)
            .event
            .owns_lease
    );
    assert!(
        !peer
            .event(RenderTestPoint::AfterLeaseRelease)
            .event
            .owns_lease
    );
    assert!(!peer.event(RenderTestPoint::OverlayEntry).event.owns_lease);
    let failed = peer.painted(first.generation);
    let stats = failed.stats.unwrap();
    assert!(stats.has_valid_scene());
    assert_eq!(
        stats.scene_progress(),
        client::render::backend::SceneProgress::Retained
    );
    let faults = render_head::workspace_stats().faults();
    assert_eq!(faults.totals()[2], before[2] + 1);
    let incident = faults.latest().unwrap();
    assert_eq!(
        incident.kind(),
        host::render::RenderFaultKind::ResourceLimit
    );
    assert_eq!(incident.reservation(), consumed.event.reservation.unwrap());
    assert!(!incident.active());

    owner.release();
    owner.painted(0);
}

#[test]
fn consume_after_idle_none_is_reported_by_quantum_plus_pump_delay() {
    let Some(_serial) = gpu_fixture() else {
        return;
    };
    let quantum = Duration::from_millis(40);
    let idle = Instant::now();
    let heartbeat = render_head::pump(idle).expect("an idle process schedules its next heartbeat");
    assert_eq!(heartbeat.instant(), idle + quantum);

    let before = render_head::workspace_stats().faults().totals();
    let mut owner = Slot::start(idle, Some(RenderTestPoint::AfterConsume));
    let consumed = owner.event(RenderTestPoint::AfterConsume);
    assert!(consumed.event.owns_lease);
    assert_eq!(consumed.at, idle);
    let detection = idle + quantum * 2;
    let next = render_head::pump(detection).expect("the overdue pump returns a future heartbeat");
    let faults = render_head::workspace_stats().faults();
    let incident = faults.latest().unwrap();
    assert_eq!(faults.totals()[1], before[1] + 1);
    assert_eq!(
        incident.kind(),
        host::render::RenderFaultKind::ConsumedOverdue
    );
    assert_eq!(incident.reservation(), consumed.event.reservation.unwrap());
    assert_eq!(incident.detected_at(), detection);
    assert!(incident.active());
    assert_eq!(next.instant(), detection + quantum);

    *owner.control.now.lock().unwrap() = detection;
    owner.release();
    owner.event(RenderTestPoint::AfterLeaseRelease);
    assert!(!render_head::workspace_stats()
        .faults()
        .latest()
        .unwrap()
        .active());
    render_head::acknowledge_fault(incident.sequence()).unwrap();
    owner.painted(0);
}
