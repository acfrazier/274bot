#![cfg(feature = "render-test-hooks")]

use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use api::identity::{SlotKey, SlotKeyMinter};
use client::client::{Client, ClientConfig, ClientRevision};
use client::dash3d::TerrainOverlayShape;
use client::render::backend::{BackendKind, GpuBackend};
use frontend_core::surface;
use host::render::RenderFaultKind;
use host::render_head::test_hooks::{
    ProcessFixture, RenderTestControl, RenderTestEvent, RenderTestPoint,
};
use host::{FrameBuf, Host, RandomClaim, ScriptRunPolicy, SlotInput, SlotRender};
use vault::ProfileSettings;

const FIXTURE_TIMEOUT: Duration = Duration::from_secs(20);
const PUMP_INTERVAL: Duration = Duration::from_millis(2);

fn serial() -> MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    let guard = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    static CONFIGURE: std::sync::Once = std::sync::Once::new();
    CONFIGURE.call_once(|| {
        host::render_head::test_hooks::configure_process(ProcessFixture {
            grant_ttl: Duration::from_millis(20),
            quantum: Duration::from_millis(40),
        })
        .expect("the bridge fixture configures the process before its first head");
    });
    guard
}

struct BotCpuOverride(Option<String>);

impl BotCpuOverride {
    fn force_gpu() -> Self {
        let previous = std::env::var("BOT_CPU").ok();
        std::env::set_var("BOT_CPU", "0");
        Self(previous)
    }
}

impl Drop for BotCpuOverride {
    fn drop(&mut self) {
        match self.0.take() {
            Some(value) => std::env::set_var("BOT_CPU", value),
            None => std::env::remove_var("BOT_CPU"),
        }
    }
}

struct ConsumeGate {
    events: Sender<RenderTestEvent>,
    releases: Mutex<Receiver<()>>,
}

impl RenderTestControl for ConsumeGate {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn at(&self, event: RenderTestEvent) {
        if event.point != RenderTestPoint::AfterConsume {
            return;
        }
        let _ = self.events.send(event);
        let _ = self
            .releases
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .recv_timeout(FIXTURE_TIMEOUT);
    }
}

struct SlotFixture {
    output: Arc<FrameBuf>,
    wake: host::SlotWake,
    events: Receiver<RenderTestEvent>,
    release: Sender<()>,
    stop: Arc<AtomicBool>,
    completed: Receiver<()>,
    worker: Option<JoinHandle<()>>,
}

impl SlotFixture {
    fn start() -> Self {
        let (wake, park) = host::wake_channel();
        let (events_tx, events) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let control = Arc::new(ConsumeGate {
            events: events_tx,
            releases: Mutex::new(release_rx),
        });
        let output = FrameBuf::new();
        let input = SlotInput::new();
        let render = SlotRender::new(slot_key(), Arc::clone(&output), Arc::new(wake.clone()))
            .with_test_control(control);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let (completed_tx, completed) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("render-fault-bridge-slot".to_owned())
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    let mut client = scene_client();
                    client.last_response = Some(Instant::now());
                    Host::run_client(
                        &mut client,
                        "t4bridge",
                        ProfileSettings::default(),
                        Arc::new(AtomicBool::new(false)),
                        Arc::new(AtomicBool::new(false)),
                        Arc::new(Mutex::new(String::new())),
                        Some(input),
                        Some(render),
                        Some(Arc::new(park)),
                        ScriptRunPolicy::default(),
                        |_, _, _, _, _| false,
                        |_| worker_stop.load(Ordering::SeqCst),
                        |_: &api::DetectedRandom| RandomClaim::Host,
                    );
                }));
                let _ = completed_tx.send(());
                if let Err(payload) = result {
                    resume_unwind(payload);
                }
            })
            .expect("the fixture slot thread starts");
        Self {
            output,
            wake,
            events,
            release,
            stop,
            completed,
            worker: Some(worker),
        }
    }

    fn consumed(&self) -> RenderTestEvent {
        self.events
            .recv_timeout(FIXTURE_TIMEOUT)
            .expect("the real slot reaches the next consumed scene")
    }

    fn release_one(&self) {
        self.release
            .send(())
            .expect("the real slot still owns its consume gate");
        self.wake.wake();
    }

    fn stop_and_join(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.release.send(());
        self.wake.wake();
        if self.worker.is_some() {
            self.completed
                .recv_timeout(FIXTURE_TIMEOUT)
                .expect("the real slot exits after its gate is released and wake is delivered");
            if let Some(worker) = self.worker.take() {
                worker
                    .join()
                    .expect("the real slot thread exits without panic");
            }
        }
    }
}

impl Drop for SlotFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.release.send(());
        self.wake.wake();
        if let Some(worker) = self.worker.take() {
            let _ = self.completed.recv_timeout(FIXTURE_TIMEOUT);
            let _ = worker.join();
        }
    }
}

struct IndependentPump {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl IndependentPump {
    fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("render-fault-bridge-pump".to_owned())
            .spawn(move || {
                while !worker_stop.load(Ordering::SeqCst) {
                    let _ = surface::pump_render(Instant::now());
                    thread::sleep(PUMP_INTERVAL);
                }
            })
            .expect("the independent render pump thread starts");
        Self {
            stop,
            worker: Some(worker),
        }
    }

    fn stop_and_join(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .expect("the independent render pump exits without panic");
        }
    }
}

impl Drop for IndependentPump {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn wait_for_overdue(expected_totals: [u64; 3]) -> host::render::RenderFault {
    let deadline = Instant::now() + FIXTURE_TIMEOUT;
    loop {
        let faults = surface::render_workspace_stats().faults();
        if faults.totals() == expected_totals {
            if let Some(fault) = faults.latest() {
                return fault;
            }
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for consumed-overdue fault"
        );
        thread::sleep(PUMP_INTERVAL);
    }
}

fn scene_client() -> Client {
    let mut client = Client::new_with_revision(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/nonexistent-t4-cache".into(),
            members: true,
            lowmem: false,
        },
        ClientRevision::R289,
    );
    client.set_draw(true);
    client.ingame = true;
    client.scene_state = 2;
    client.world.fill_base_level(0);
    for x in 8..24 {
        for z in 8..40 {
            let hsl = ((x * 7 + z * 13) % 60) * 1024 + 2624;
            client.world.set_ground(
                0,
                x,
                z,
                TerrainOverlayShape::PLAIN,
                0,
                -1,
                0,
                0,
                0,
                0,
                hsl,
                hsl,
                hsl,
                hsl,
                0,
                0,
                0,
                0,
                0x00ff00,
                0,
            );
        }
    }
    client.cam_x = 16 * 128;
    client.cam_z = 4 * 128;
    client.cam_y = -600;
    client.cam_pitch = 200;
    client.cam_yaw = 0;
    client
}

fn slot_key() -> SlotKey {
    static SLOTS: SlotKeyMinter = SlotKeyMinter::new();
    SLOTS
        .mint()
        .expect("the fixture receives a unique slot key")
}

fn one_more_overdue(mut totals: [u64; 3]) -> [u64; 3] {
    totals[1] += 1;
    totals
}

#[test]
fn panel_fault_bridge_acknowledges_only_the_selected_incident() {
    let _serial = serial();
    let _gpu_request = BotCpuOverride::force_gpu();
    if let Err(error) = GpuBackend::try_new() {
        assert_ne!(
            std::env::var("T4_REQUIRE_GPU").as_deref(),
            Ok("1"),
            "T4_REQUIRE_GPU=1 but GPU initialization failed: {error}"
        );
        eprintln!("GPU adapter unavailable; render fault bridge GPU test skipped: {error}");
        return;
    }

    let baseline = surface::render_workspace_stats().faults();
    let first_totals = one_more_overdue(baseline.totals());
    let mut slot = SlotFixture::start();
    let first_consumed = slot.consumed();
    assert_eq!(first_consumed.point, RenderTestPoint::AfterConsume);
    assert!(
        first_consumed.owns_lease,
        "the first consume remains on its real slot"
    );
    let first_head = slot
        .output
        .fair_stats()
        .expect("the feature exposes real slot head statistics");
    assert_eq!(first_head.backend(), BackendKind::Gpu);
    assert_eq!(
        first_head.scene_consumes(),
        0,
        "the held paint has not published its completed-paint statistics"
    );

    let mut pump = IndependentPump::start();
    let first_fault = wait_for_overdue(first_totals);
    assert_eq!(first_fault.kind(), RenderFaultKind::ConsumedOverdue);
    assert_eq!(first_fault.reservation().head(), first_consumed.head);
    assert!(first_fault.active());
    surface::acknowledge_render_fault(first_fault.sequence())
        .expect("the surface forwards acknowledgement of the selected incident");
    let acknowledged_active = surface::render_workspace_stats().faults();
    assert_eq!(
        acknowledged_active.acknowledged_sequence(),
        first_fault.sequence()
    );
    assert_eq!(acknowledged_active.totals(), first_totals);
    assert!(
        acknowledged_active
            .latest()
            .is_some_and(|fault| fault.active()),
        "acknowledgement does not hide the still-owned workspace fault"
    );

    slot.release_one();
    let second_consumed = slot.consumed();
    assert_eq!(second_consumed.point, RenderTestPoint::AfterConsume);
    assert_eq!(second_consumed.head, first_consumed.head);
    assert!(second_consumed.owns_lease);
    let second_head = slot
        .output
        .fair_stats()
        .expect("the first completed paint publishes updated head statistics");
    assert_eq!(second_head.backend(), BackendKind::Gpu);
    assert_eq!(
        second_head.scene_consumes(),
        1,
        "the second consume event does not publish an incomplete paint"
    );

    let second_totals = one_more_overdue(first_totals);
    let second_fault = wait_for_overdue(second_totals);
    assert_eq!(second_fault.kind(), RenderFaultKind::ConsumedOverdue);
    assert_eq!(second_fault.reservation().head(), second_consumed.head);
    assert!(second_fault.sequence() > first_fault.sequence());
    assert!(second_fault.active());
    surface::acknowledge_render_fault(first_fault.sequence())
        .expect("an older acknowledgement remains a valid selected incident");
    let after_old_ack = surface::render_workspace_stats().faults();
    assert_eq!(
        after_old_ack.acknowledged_sequence(),
        first_fault.sequence()
    );
    assert_eq!(after_old_ack.totals(), second_totals);
    assert_eq!(
        after_old_ack.latest().map(|fault| fault.sequence()),
        Some(second_fault.sequence())
    );
    assert!(
        after_old_ack.latest().is_some_and(|fault| fault.active()),
        "the old acknowledgement cannot clear the newer owned incident"
    );

    slot.stop_and_join();
    pump.stop_and_join();
    let released = surface::render_workspace_stats().faults();
    assert_eq!(
        released.totals(),
        second_totals,
        "fault totals persist after release"
    );
    assert_eq!(released.acknowledged_sequence(), first_fault.sequence());
    assert!(released.latest().is_some_and(|fault| !fault.active()));
}
