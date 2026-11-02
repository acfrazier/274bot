//! The Fleet window's levels-gained column: the observe pump feeds each run's
//! level baseline from game-ready snapshots only, and gains count from the
//! first of them.
use super::*;

/// Test-only trace for the TLA+ trace validation in `tools/hold-model-tla`:
/// the page-click machine records its own lease events here, and
/// `HoldRig::run` writes one JSON line per frame to
/// `$HOLD_TRACE_DIR/holdrig-<test thread>.jsonl` when that variable is set.
thread_local! {
    static OWNER_EVENTS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn trace_owner(event: String) {
    OWNER_EVENTS.with(|events| events.borrow_mut().push(event));
}

fn observe(
    c: &mut Client,
    snap: &GameSnapshot,
    ingame_here: bool,
    scripts: &ScriptWall,
    cheats: &Arc<Mutex<HashMap<String, VecDeque<String>>>>,
) {
    let (navs, world) = empty_nav();
    let names = api::obj_names::ObjNames::from_objs(&c.cache.objs);
    let inv: [(i32, i32); 0] = [];
    script_observe(
        c,
        "alice",
        true,
        true,
        1,
        ingame_here.then_some((3205, 3205, 0)),
        Some(&inv),
        None,
        Some(snap),
        Some(&names),
        scripts,
        cheats,
        &navs,
        &world,
        false,
        false,
    );
}

fn levels_gained(scripts: &ScriptWall) -> Option<u32> {
    script_slot(scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .progress(Instant::now())
        .map(|progress| progress.levels_gained())
}

#[test]
fn levels_gained_count_from_the_first_game_ready_observation_of_a_run() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_load_settled(
            "export default class T extends LoopingBot { async loop() {} }".to_string(),
            script_js::LoadShape::CompatClass,
            vec![],
        )
        .expect("isolate starts");

    let mut c = bank_fetch_client();
    c.stat_base_level[8] = 30;
    c.stat_base_level[10] = 40;
    c.gens.stat += 1;
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);

    // No player position yet: not game-ready, so nothing is baselined.
    observe(&mut c, &snap, false, &scripts, &cheats);
    assert_eq!(levels_gained(&scripts), Some(0));

    // The first ready observation fixes the baseline (30 and 40).
    observe(&mut c, &snap, true, &scripts, &cheats);
    c.stat_base_level[8] = 31;
    c.stat_base_level[10] = 42;
    c.gens.stat += 1;
    snap.rebuild(&c);
    observe(&mut c, &snap, true, &scripts, &cheats);
    assert_eq!(
        levels_gained(&scripts),
        Some(3),
        "+1 and +2 since the baseline"
    );

    // A level lost to a drain or a relog cannot make a negative gain.
    c.stat_base_level[8] = 20;
    c.gens.stat += 1;
    snap.rebuild(&c);
    observe(&mut c, &snap, true, &scripts, &cheats);
    assert_eq!(levels_gained(&scripts), Some(2));
}

struct QueueBankButton;

impl script::native::Script for QueueBankButton {
    fn tick(
        &mut self,
        ctx: &mut script::native::NativeTick<'_>,
    ) -> Result<script::native::ScriptFlow, script::native::ScriptFailure> {
        ctx.queue_test_interaction(script::shim::InteractReq::IfButton { component_id: 601 });
        Ok(script::native::ScriptFlow::Continue)
    }
}

/// Run one observe with a fresh policy and report whether the host would hold
/// its press after the script's interaction of that frame.
fn holds_after_one_interaction(script: Box<dyn script::native::Script>) -> bool {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (navs, world) = empty_nav();
    let mut client = bank_fetch_client();
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(&client);
    let names = Arc::new(api::obj_names::ObjNames::from_objs(&client.cache.objs));
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_test_script(script, None)
        .expect("native script starts");

    let cache = Arc::clone(&client.cache);
    let inv: [(i32, i32); 0] = [];
    let mut run_policy = host::ScriptRunPolicy::default();
    script_observe_cached(
        &mut client,
        "alice",
        true,
        true,
        1,
        Some((3205, 3205, 0)),
        Some(&inv),
        None,
        Some(&snapshot),
        None,
        Some(names.as_ref()),
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
        None,
        None,
        Some(cache),
        Some(Arc::clone(&names)),
        Some(&mut run_policy),
    );

    assert!(client.out.pos > 0, "the script sends its interface action");
    assert_eq!(
        client.out.data()[0],
        client::io::ClientProt::IF_BUTTON.id as u8
    );
    run_policy.should_defer_auto_run()
}

#[test]
fn an_interaction_without_a_wait_never_holds_auto_run() {
    assert!(
        !holds_after_one_interaction(Box::new(QueueBankButton)),
        "a raw script interaction that expects no modal takes no hold"
    );
    assert!(
        !holds_after_one_interaction(Box::new(QueueNativeButton { handle: None })),
        "a native interaction without a page wait takes no hold"
    );
}

struct QueueNativeButton {
    handle: Option<script::native::ActionHandle<NativeButtonMachine>>,
}

struct NativeButtonMachine;

impl script::native::NativeMachine for NativeButtonMachine {
    type Args = ();
    type Output = ();

    fn begin(
        _args: Self::Args,
        cx: &mut script::native::ActionContext<'_>,
    ) -> Result<Self, script::native::ActionError> {
        cx.emit(script::shim::InteractReq::IfButton { component_id: 601 })?;
        Ok(Self)
    }

    fn poll(
        &mut self,
        _cx: &mut script::native::ActionContext<'_>,
    ) -> std::task::Poll<Result<Self::Output, script::native::ActionError>> {
        std::task::Poll::Pending
    }
}

impl script::native::Script for QueueNativeButton {
    fn tick(
        &mut self,
        ctx: &mut script::native::NativeTick<'_>,
    ) -> Result<script::native::ScriptFlow, script::native::ScriptFailure> {
        if self.handle.is_none() {
            self.handle = Some(
                ctx.actions
                    .begin::<NativeButtonMachine>((), &mut ctx.cx)
                    .expect("native action starts"),
            );
        }
        Ok(script::native::ScriptFlow::Continue)
    }
}

/// The script's own content-timed bound for its page, in observed ticks.
/// When it expires, the wait fails and the script records the failure.
const PAGE_WAIT_TICKS: u64 = 12;

/// A journal-style click on the production path: one `IfButton`, and a
/// modal-wait lease from the click until the script observes its page or its
/// own bound expires. The lease carries that bound. Afterwards it stays
/// pending, as the journal machine does while it closes the page. A `hung`
/// machine never checks its bound: its owner stopped running its wait.
///
/// With `active_bound`, the bound is on the owner's active clock, as a
/// `Dialogue` page or acknowledgement wait is (`cx.active_now()`, which
/// Pause stops). With `ack`, the click is a send on an open page: the wait
/// ends only at a page seen after that page closed (its acknowledgement).
struct PageClickMachine {
    page: Arc<AtomicBool>,
    wait: Option<script::native::ModalWaitLease>,
    deadline: u64,
    active_deadline: Option<std::time::Duration>,
    seen_closed: bool,
    hung: bool,
}

#[derive(Clone)]
struct PageClickArgs {
    page: Arc<AtomicBool>,
    wait_ticks: u64,
    active_bound: Option<std::time::Duration>,
    ack: bool,
    hung: bool,
}

impl script::native::NativeMachine for PageClickMachine {
    type Args = PageClickArgs;
    type Output = ();

    fn begin(
        args: Self::Args,
        cx: &mut script::native::ActionContext<'_>,
    ) -> Result<Self, script::native::ActionError> {
        cx.emit(script::shim::InteractReq::IfButton { component_id: 601 })?;
        let deadline = cx.evidence().tick.saturating_add(args.wait_ticks);
        let active_deadline = args.active_bound.map(|bound| cx.active_now() + bound);
        let lease = match active_deadline {
            Some(at) => script::native::WaitDeadline::Active(at),
            None => script::native::WaitDeadline::Tick(deadline),
        };
        trace_owner(match args.active_bound {
            Some(bound) => format!(
                "{{\"ev\":\"begin\",\"k\":\"A\",\"ms\":{},\"ack\":{},\"hung\":{}}}",
                bound.as_millis(),
                args.ack,
                args.hung
            ),
            None => format!(
                "{{\"ev\":\"begin\",\"k\":\"T\",\"ticks\":{},\"ack\":{},\"hung\":{}}}",
                args.wait_ticks, args.ack, args.hung
            ),
        });
        Ok(Self {
            page: args.page,
            wait: Some(cx.begin_modal_wait(lease)?),
            deadline,
            active_deadline,
            seen_closed: !args.ack,
            hung: args.hung,
        })
    }

    fn poll(
        &mut self,
        cx: &mut script::native::ActionContext<'_>,
    ) -> std::task::Poll<Result<Self::Output, script::native::ActionError>> {
        if self.hung {
            return std::task::Poll::Pending;
        }
        if self.page.load(Ordering::Relaxed) {
            if self.wait.is_some() && self.seen_closed {
                // The script observed its page: its wait ends here.
                self.wait = None;
                trace_owner("{\"ev\":\"end\",\"why\":\"page\"}".to_owned());
            }
        } else {
            self.seen_closed = true;
        }
        let expired = match self.active_deadline {
            Some(at) => cx.active_now() >= at,
            None => cx.evidence().tick >= self.deadline,
        };
        if self.wait.is_some() && expired {
            trace_owner("{\"ev\":\"end\",\"why\":\"expired\"}".to_owned());
            return std::task::Poll::Ready(Err(script::native::ActionError::Failed(Arc::from(
                "page wait expired",
            ))));
        }
        std::task::Poll::Pending
    }
}

impl Drop for PageClickMachine {
    fn drop(&mut self) {
        if self.wait.is_some() {
            trace_owner("{\"ev\":\"drop\"}".to_owned());
        }
    }
}

/// The script clicks once when armed and polls its machine on each tick.
struct ClickWhenArmed {
    armed: Arc<AtomicBool>,
    args: PageClickArgs,
    expired: Arc<AtomicBool>,
    handle: Option<script::native::ActionHandle<PageClickMachine>>,
}

impl script::native::Script for ClickWhenArmed {
    fn tick(
        &mut self,
        ctx: &mut script::native::NativeTick<'_>,
    ) -> Result<script::native::ScriptFlow, script::native::ScriptFailure> {
        if let Some(handle) = &self.handle {
            if let std::task::Poll::Ready(result) = ctx.actions.poll(handle, &mut ctx.cx) {
                if result.is_err() {
                    self.expired.store(true, Ordering::Relaxed);
                }
                self.handle = None;
            }
        }
        if self.handle.is_none() && self.armed.swap(false, Ordering::Relaxed) {
            self.handle = Some(
                ctx.actions
                    .begin::<PageClickMachine>(self.args.clone(), &mut ctx.cx)
                    .expect("native action starts"),
            );
        }
        Ok(script::native::ScriptFlow::Continue)
    }
}

/// One host frame: `drain` PLAYER_INFO packets arrive with `modal` as the
/// client's main modal, the script clicks on this frame when `click`, and
/// the operator stops, pauses or resumes the script before this frame when
/// `stop`, `pause` or `resume`. `sleep_ms` of wall time pass before it.
#[derive(Clone, Copy)]
struct Frame {
    drain: u32,
    modal: i32,
    click: bool,
    stop: bool,
    pause: bool,
    resume: bool,
    sleep_ms: u64,
}

const JOURNAL_ROOT: i32 = 8134;

fn frame(drain: u32, modal: i32) -> Frame {
    Frame {
        drain,
        modal,
        click: false,
        stop: false,
        pause: false,
        resume: false,
        sleep_ms: 0,
    }
}

fn click_frame(drain: u32, modal: i32) -> Frame {
    Frame {
        click: true,
        ..frame(drain, modal)
    }
}

fn stop_frame(drain: u32, modal: i32) -> Frame {
    Frame {
        stop: true,
        ..frame(drain, modal)
    }
}

/// The production pairing: `Host::run_client` owns the host snapshot and the
/// auto-run decision, and `script_observe_cached` runs the script, reads its
/// wait and hands it to the host policy in the same frame, before the press.
struct HoldRig {
    scripts: ScriptWall,
    cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    navs: EmptyNav,
    names: Arc<api::obj_names::ObjNames>,
    snapshot: Arc<GameSnapshot>,
    client: Client,
    armed: Arc<AtomicBool>,
    page: Arc<AtomicBool>,
    expired: Arc<AtomicBool>,
    /// The script's `expired` record after each frame of the last run.
    expired_after: Arc<Mutex<Vec<bool>>>,
    script_tick: Arc<std::sync::atomic::AtomicU64>,
    run_policy: Option<host::ScriptRunPolicy>,
}

impl HoldRig {
    fn new() -> Self {
        Self::with_wait(PAGE_WAIT_TICKS, false)
    }

    /// A rig whose click waits `wait_ticks` for its page; a `hung` owner
    /// never checks that bound.
    fn with_wait(wait_ticks: u64, hung: bool) -> Self {
        Self::with_args(wait_ticks, None, false, hung)
    }

    /// A rig whose click waits `bound` of the owner's active time: a page
    /// wait, or with `ack` an acknowledgement wait.
    fn with_active_wait(bound: std::time::Duration, ack: bool) -> Self {
        Self::with_args(u64::MAX, Some(bound), ack, false)
    }

    fn with_args(
        wait_ticks: u64,
        active_bound: Option<std::time::Duration>,
        ack: bool,
        hung: bool,
    ) -> Self {
        let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
        let cheats = Arc::new(Mutex::new(HashMap::new()));
        let mut client = bank_fetch_client();
        // The script's own view is the fixture's: the click's component (601)
        // is visible only under the fixture's bank. The host reads the
        // client's modal roots, which the rig then clears and drives itself.
        // The script observes the page through `page`, set from the same
        // frame's modal.
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(&client);
        client.main_modal_id = -1;
        client.chat_modal_id = -1;
        client.side_modal_id = -1;
        client.runenergy = 20;
        let names = Arc::new(api::obj_names::ObjNames::from_objs(&client.cache.objs));
        let armed = Arc::new(AtomicBool::new(false));
        let page = Arc::new(AtomicBool::new(false));
        let expired = Arc::new(AtomicBool::new(false));
        script_slot_or_insert(&scripts, "alice")
            .lock()
            .unwrap()
            .start_test_script(
                Box::new(ClickWhenArmed {
                    armed: Arc::clone(&armed),
                    args: PageClickArgs {
                        page: Arc::clone(&page),
                        wait_ticks,
                        active_bound,
                        ack,
                        hung,
                    },
                    expired: Arc::clone(&expired),
                    handle: None,
                }),
                None,
            )
            .expect("native script starts");
        Self {
            scripts,
            cheats,
            navs: empty_nav(),
            names,
            snapshot: Arc::new(snapshot),
            client,
            armed,
            page,
            expired,
            expired_after: Arc::new(Mutex::new(Vec::new())),
            script_tick: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            run_policy: Some(host::ScriptRunPolicy::default()),
        }
    }

    /// Run `frames` through one `Host::run_client` call, then one empty frame
    /// that shows the last frame's press. Returns the host's cumulative
    /// accepted auto-run presses after each of `frames`.
    fn run(&mut self, frames: &[Frame]) -> Vec<u32> {
        let plan: Arc<Vec<Frame>> = Arc::new(
            frames
                .iter()
                .copied()
                .chain([frame(0, frames.last().map_or(-1, |last| last.modal))])
                .collect(),
        );
        let index = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
        let tick_edge = Arc::new(AtomicBool::new(false));

        let observe_seen = Arc::clone(&seen);
        let observe_index = Arc::clone(&index);
        let observe_edge = Arc::clone(&tick_edge);
        let observe_tick = Arc::clone(&self.script_tick);
        let observe_scripts = Arc::clone(&self.scripts);
        let observe_cheats = Arc::clone(&self.cheats);
        let (observe_navs, observe_world) = (Arc::clone(&self.navs.0), self.navs.1.clone());
        let observe_names = Arc::clone(&self.names);
        let observe_snapshot = Arc::clone(&self.snapshot);
        let observe_cache = Arc::clone(&self.client.cache);
        let observe_expired = Arc::clone(&self.expired);
        let observe_expired_after = Arc::clone(&self.expired_after);
        observe_expired_after.lock().unwrap().clear();
        let frame_log: Arc<Mutex<Vec<(u64, bool, Vec<String>)>>> = Arc::default();
        let observe_log = Arc::clone(&frame_log);

        let probe_plan = Arc::clone(&plan);
        let probe_index = Arc::clone(&index);
        let probe_edge = Arc::clone(&tick_edge);
        let probe_armed = Arc::clone(&self.armed);
        let probe_page = Arc::clone(&self.page);
        let probe_scripts = Arc::clone(&self.scripts);

        let run_policy = Host::run_client(
            &mut self.client,
            "alice",
            vault::ProfileSettings::default(),
            Arc::new(AtomicBool::new(true)),
            Arc::new(AtomicBool::new(true)),
            Arc::new(Mutex::new("strength".to_string())),
            None,
            None,
            None,
            self.run_policy.take().unwrap_or_default(),
            move |c, _, run_sends, _, run_policy| {
                observe_seen.lock().unwrap().push(run_sends);
                let edge = observe_edge.load(Ordering::Relaxed);
                let tick = if edge {
                    observe_tick.fetch_add(1, Ordering::Relaxed) + 1
                } else {
                    observe_tick.load(Ordering::Relaxed)
                };
                let inv: [(i32, i32); 0] = [];
                script_observe_cached(
                    c,
                    "alice",
                    true,
                    edge,
                    tick,
                    Some((3205, 3205, 0)),
                    Some(&inv),
                    None,
                    Some(observe_snapshot.as_ref()),
                    None,
                    Some(observe_names.as_ref()),
                    &observe_scripts,
                    &observe_cheats,
                    &observe_navs,
                    &observe_world,
                    false,
                    false,
                    None,
                    None,
                    Some(Arc::clone(&observe_cache)),
                    Some(Arc::clone(&observe_names)),
                    Some(&mut *run_policy),
                );
                observe_expired_after
                    .lock()
                    .unwrap()
                    .push(observe_expired.load(Ordering::Relaxed));
                observe_log.lock().unwrap().push((
                    tick,
                    run_policy.should_defer_auto_run(),
                    OWNER_EVENTS.with(|events| std::mem::take(&mut *events.borrow_mut())),
                ));
                observe_index.fetch_add(1, Ordering::Relaxed);
                true
            },
            move |c| {
                let Some(step) = probe_plan.get(probe_index.load(Ordering::Relaxed)) else {
                    return true;
                };
                c.gens.player_info += u64::from(step.drain);
                if step.drain > 0 {
                    c.gens.player += 1;
                }
                c.main_modal_id = step.modal;
                if step.sleep_ms > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(step.sleep_ms));
                }
                if step.stop || step.pause || step.resume {
                    let slot = script_slot(&probe_scripts, "alice").unwrap();
                    let mut slot = slot.lock().unwrap();
                    if step.stop {
                        slot.stop();
                    }
                    if step.pause {
                        slot.pause();
                    }
                    if step.resume {
                        slot.resume();
                    }
                }
                probe_page.store(step.modal != -1, Ordering::Relaxed);
                probe_edge.store(step.drain > 0, Ordering::Relaxed);
                probe_armed.store(step.click, Ordering::Relaxed);
                false
            },
            |_: &api::DetectedRandom| RandomClaim::Host,
        );
        self.run_policy = Some(run_policy);
        // `seen[i + 1]` is the count the host held when frame `i + 1` began,
        // that is, after frame `i`'s press.
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), frames.len() + 1, "every frame was observed");
        if let Some(dir) = std::env::var_os("HOLD_TRACE_DIR") {
            use std::io::Write;
            let name = std::thread::current()
                .name()
                .unwrap_or("unnamed")
                .replace("::", ".");
            let path = std::path::Path::new(&dir).join(format!("holdrig-{name}.jsonl"));
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .expect("the trace file opens");
            let log = frame_log.lock().unwrap();
            for (i, step) in frames.iter().enumerate() {
                let (tick, eff, owner) = &log[i];
                writeln!(
                    file,
                    "{{\"i\":{i},\"drain\":{},\"edge\":{},\"modal\":{},\"click\":{},\"stop\":{},\"pause\":{},\"resume\":{},\"sleep_ms\":{},\"tick\":{tick},\"eff\":{eff},\"press\":{},\"owner\":[{}]}}",
                    step.drain,
                    step.drain > 0,
                    step.modal != -1,
                    step.click,
                    step.stop,
                    step.pause,
                    step.resume,
                    step.sleep_ms,
                    seen[i + 1] > seen[i],
                    owner.join(",")
                )
                .expect("the trace line is written");
            }
        }
        seen[1..].to_vec()
    }
}

#[test]
fn auto_run_hold_waits_for_the_page_and_its_close() {
    let mut rig = HoldRig::new();
    let presses = rig.run(&[
        click_frame(1, -1),
        frame(1, JOURNAL_ROOT),
        frame(1, JOURNAL_ROOT),
        frame(1, -1),
        frame(1, -1),
    ]);
    assert_eq!(
        presses,
        [0, 0, 0, 1, 1],
        "no press while the script waits or the page is up; one after the close"
    );
}

/// R3-1: several updates queue before the click's page. The hold counts no
/// updates, so it lasts until the script sees its page.
#[test]
fn auto_run_hold_survives_queued_updates_before_the_page() {
    let mut rig = HoldRig::new();
    let presses = rig.run(&[
        click_frame(1, -1),
        frame(1, -1),
        frame(1, -1),
        frame(1, -1),
        frame(1, -1),
        frame(1, -1),
        frame(1, -1),
        frame(1, JOURNAL_ROOT),
        frame(1, -1),
    ]);
    assert_eq!(
        presses,
        [0, 0, 0, 0, 0, 0, 0, 0, 1],
        "six updates without the page do not release the press"
    );
}

/// R2-2 with queued history: the modal arrives in a later drain than its
/// cycle's PLAYER_INFO, after three updates without it.
#[test]
fn auto_run_hold_survives_a_modal_in_a_later_drain_than_its_player_info() {
    let mut rig = HoldRig::new();
    let presses = rig.run(&[
        click_frame(1, -1),
        frame(1, -1),
        frame(1, -1),
        frame(1, -1),
        frame(0, JOURNAL_ROOT),
        frame(1, JOURNAL_ROOT),
        frame(1, -1),
    ]);
    assert_eq!(
        presses,
        [0, 0, 0, 0, 0, 0, 1],
        "no press before the modal's drain, none while the page is up, one after the close"
    );
}

/// The client decodes five packets per frame: a long queue arrives as full
/// five-packet drains. They do not release the press.
#[test]
fn auto_run_hold_survives_full_five_packet_drains() {
    let mut rig = HoldRig::new();
    let presses = rig.run(&[
        click_frame(1, -1),
        frame(5, -1),
        frame(5, -1),
        frame(0, JOURNAL_ROOT),
        frame(5, JOURNAL_ROOT),
        frame(5, -1),
    ]);
    assert_eq!(presses, [0, 0, 0, 0, 0, 1]);
}

#[test]
fn auto_run_hold_ends_when_the_script_wait_expires() {
    let mut rig = HoldRig::new();
    let mut frames = vec![click_frame(1, -1)];
    frames.extend(std::iter::repeat_n(
        frame(1, -1),
        PAGE_WAIT_TICKS as usize + 1,
    ));
    let presses = rig.run(&frames);
    let expiry = PAGE_WAIT_TICKS as usize;
    assert!(
        presses[..expiry].iter().all(|&count| count == 0),
        "held until the script's own bound: {presses:?}"
    );
    assert_eq!(
        presses[expiry], 1,
        "the expired wait ends the hold on its own frame"
    );
    assert!(
        rig.expired.load(Ordering::Relaxed),
        "the script recorded its failed wait"
    );
}

#[test]
fn auto_run_hold_ends_when_the_script_is_stopped() {
    let mut rig = HoldRig::new();
    assert_eq!(
        rig.run(&[click_frame(1, -1), frame(1, -1), frame(1, -1)]),
        [0, 0, 0],
        "the control: the hold lasts while the script waits"
    );
    let mut rig = HoldRig::new();
    assert_eq!(
        rig.run(&[click_frame(1, -1), frame(1, -1), stop_frame(1, -1)]),
        [0, 0, 1],
        "a dropped owner releases the hold on its own frame"
    );
}

/// R4S-1 on the production path: a valid 30-tick quiet wait (the longest
/// authored end gap, `MAX_GAP_TICKS`) holds the press to its own deadline.
/// A fixed host limit shorter than the wait (the old 25 ticks) fails here.
#[test]
fn auto_run_hold_lasts_a_thirty_tick_wait_to_its_deadline() {
    const GAP_TICKS: u64 = 30;
    let mut rig = HoldRig::with_wait(GAP_TICKS, false);
    let mut frames = vec![click_frame(1, -1)];
    frames.extend(std::iter::repeat_n(frame(1, -1), GAP_TICKS as usize));
    let presses = rig.run(&frames);
    let deadline = GAP_TICKS as usize;
    assert!(
        presses[..deadline].iter().all(|&count| count == 0),
        "held for the whole valid wait: {presses:?}"
    );
    assert_eq!(
        presses[deadline], 1,
        "the script ends its wait on its deadline tick"
    );
}

/// A lease whose owner stops checking its own bound is released one margin
/// after its deadline, not before (the host logs that release as a failure;
/// `host` `lib_tests.rs` checks the logged record).
#[test]
fn auto_run_hold_of_a_hung_owner_ends_after_its_deadline_plus_the_margin() {
    let mut rig = HoldRig::with_wait(PAGE_WAIT_TICKS, true);
    let mut frames = vec![click_frame(1, -1)];
    frames.extend(std::iter::repeat_n(
        frame(1, -1),
        (PAGE_WAIT_TICKS + host::SCRIPT_WAIT_MARGIN_TICKS) as usize,
    ));
    let presses = rig.run(&frames);
    let release = (PAGE_WAIT_TICKS + host::SCRIPT_WAIT_MARGIN_TICKS) as usize;
    assert!(
        presses[..release].iter().all(|&count| count == 0),
        "held to the deadline and its margin: {presses:?}"
    );
    assert_eq!(presses[release], 1, "then the host ends the hung hold");
    assert!(
        !rig.expired.load(Ordering::Relaxed),
        "the hung owner never ended its own wait"
    );
}

/// The owner's active-clock bound in the paused-wait tests below.
const ACTIVE_BOUND: std::time::Duration = std::time::Duration::from_millis(500);
/// The pause: 12 frames of 100 ms wall time, more than twice the bound, with
/// game ticks still observed on every frame.
const PAUSE_FRAMES: usize = 12;
const PAUSE_FRAME_MS: u64 = 100;
/// After Resume: frames of 25 ms until the owner's own bound ends its wait.
const RESUMED_FRAMES: usize = 60;
const RESUMED_FRAME_MS: u64 = 25;

/// `before`, then Pause, the pause, Resume and the resumed frames, all with
/// main modal `modal`. Returns the frames and the index of the Resume frame.
fn paused_wait_frames(before: &[Frame], modal: i32) -> (Vec<Frame>, usize) {
    let mut frames = before.to_vec();
    frames.push(Frame {
        pause: true,
        ..frame(1, modal)
    });
    frames.extend(std::iter::repeat_n(
        Frame {
            sleep_ms: PAUSE_FRAME_MS,
            ..frame(1, modal)
        },
        PAUSE_FRAMES,
    ));
    let resume = frames.len();
    frames.push(Frame {
        resume: true,
        ..frame(1, modal)
    });
    frames.extend(std::iter::repeat_n(
        Frame {
            sleep_ms: RESUMED_FRAME_MS,
            ..frame(1, modal)
        },
        RESUMED_FRAMES,
    ));
    (frames, resume)
}

/// No press from the click to the frame where the owner ends its wait on its
/// own active-clock bound, which is after Resume; the press on that frame.
fn assert_held_until_the_owner_ends_its_wait(rig: &HoldRig, presses: &[u32], resume: usize) {
    let expired_after = rig.expired_after.lock().unwrap().clone();
    let end = expired_after
        .iter()
        .position(|&expired| expired)
        .expect("the owner's own bound ends its wait after Resume");
    assert!(
        end > resume,
        "the pause did not spend the owner's bound: it ended its wait on frame {end}, \
         Resume was frame {resume}"
    );
    assert!(
        presses[..end].iter().all(|&count| count == 0),
        "held for the whole pause and to the owner's own deadline: {presses:?}"
    );
    assert_eq!(
        presses[end], 1,
        "the press goes out on the frame where the owner ends its wait"
    );
}

/// R4S-1, active clock, on the production frame path: a page wait whose
/// owner is paused (real `SlotScript::pause`) for longer than its bound
/// keeps the hold for the whole pause, while the host samples it on every
/// frame. After Resume, the owner's own active-clock deadline decides.
/// Converting the bound to wall time when the lease is taken fails here.
#[test]
fn auto_run_hold_of_a_paused_page_wait_lasts_to_the_owners_own_deadline() {
    let mut rig = HoldRig::with_active_wait(ACTIVE_BOUND, false);
    let (frames, resume) = paused_wait_frames(&[click_frame(1, -1), frame(1, -1)], -1);
    let presses = rig.run(&frames);
    assert_held_until_the_owner_ends_its_wait(&rig, &presses, resume);
}

/// The same for an acknowledgement wait: the click is a send on an open
/// page, the page closes, and the script waits for its acknowledgement with
/// no modal up, so only its hold keeps the press back.
#[test]
fn auto_run_hold_of_a_paused_acknowledgement_wait_lasts_to_the_owners_own_deadline() {
    let mut rig = HoldRig::with_active_wait(ACTIVE_BOUND, true);
    let (frames, resume) = paused_wait_frames(
        &[
            click_frame(1, JOURNAL_ROOT),
            frame(1, JOURNAL_ROOT),
            frame(1, -1),
        ],
        -1,
    );
    let presses = rig.run(&frames);
    assert_held_until_the_owner_ends_its_wait(&rig, &presses, resume);
}

/// R4S-2: each compatibility journal read has its own key (token and work
/// generation). A read that begins at once after the previous one, with no
/// host sample of a free lease between them, gets a fresh hold with its own
/// deadline: it inherits neither the previous read's hold nor its release.
/// A constant key for every read fails here.
#[test]
fn compat_journal_read_to_read_hand_off_takes_a_fresh_hold() {
    use script::surface::JournalWait;
    let mut slot = script::SlotScript::new();
    slot.bind_incarnation(44);
    let factory = Arc::new(script::surface::test_support::StubSurfaceFactory::new());
    slot.bind_surface_factory(factory.clone());
    slot.start_surface(script::surface::SurfaceStart {
        card: Arc::new(script::surface::test_support::StubSurfaceCard),
        settings_bag: None,
        game_data: None,
        named_banks: Arc::default(),
        loadouts: Arc::default(),
    })
    .expect("start the compatibility surface");
    let run = factory.last_run().expect("stub surface run");
    run.set_readiness(script::surface::SurfaceReady::Ready);
    assert_eq!(
        slot.poll_start(),
        script::StartPoll::Settled(script::StartOutcome::Ready)
    );
    let mut policy = host::ScriptRunPolicy::default();
    // The production seam (`script_observe.rs`): the slot's wait, then the
    // host policy, on the frame's tick.
    let mut sample = |slot: &mut script::SlotScript, now: Instant, tick: u64| {
        let wait = slot
            .script_interaction_wait(now, tick)
            .map(|wait| host::ScriptWait {
                key: wait.key,
                deadline_tick: wait.deadline_tick,
            });
        let expired = policy.observe_script_wait(wait, tick);
        let held = policy.should_defer_auto_run();
        if let Some(dir) = std::env::var_os("HOLD_TRACE_DIR") {
            use std::io::Write;
            let name = std::thread::current()
                .name()
                .unwrap_or("unnamed")
                .replace("::", ".");
            let path = std::path::Path::new(&dir).join(format!("compat-{name}.jsonl"));
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .expect("the trace file opens");
            writeln!(
                file,
                "{{\"ev\":\"sample\",\"wait\":{},\"tick\":{tick},\"expired\":{},\"held\":{held}}}",
                wait.map_or("null".to_owned(), |wait| format!(
                    "{{\"key\":{},\"dl\":{}}}",
                    wait.key, wait.deadline_tick
                )),
                expired.is_some()
            )
            .expect("the trace line is written");
        }
        (expired, held)
    };
    let start = Instant::now();
    let bound = std::time::Duration::from_secs(10);
    run.set_journal_wait(Some(JournalWait {
        token: 1,
        generation: 3,
        deadline: start + bound,
    }));
    assert_eq!(
        sample(&mut slot, start, 1),
        (None, true),
        "the first read holds"
    );
    // The first read is still reported after its own bound (a stuck read):
    // the host ends that hold as a logged failure.
    let late = start + bound;
    let (expired, held) = sample(&mut slot, late, 2);
    assert!(expired.is_some() && !held, "the stuck read's hold ends");
    // At once, with no free sample between, the next read begins.
    run.set_journal_wait(Some(JournalWait {
        token: 2,
        generation: 3,
        deadline: late + bound,
    }));
    for tick in 2..=40 {
        assert_eq!(
            sample(&mut slot, late, tick),
            (None, true),
            "the second read holds on tick {tick} with its own bound"
        );
    }
    let (expired, held) = sample(&mut slot, late + bound, 41);
    assert!(
        expired.is_some() && !held,
        "the second read's own bound ends it"
    );
    // A new work generation with the same token is also a new read.
    run.set_journal_wait(Some(JournalWait {
        token: 2,
        generation: 4,
        deadline: late + bound + bound,
    }));
    assert_eq!(sample(&mut slot, late + bound, 42), (None, true));
    slot.stop();
}
