//! The Fleet window's levels-gained column: the observe pump feeds each run's
//! level baseline from game-ready snapshots only, and gains count from the
//! first of them.
use super::*;

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

#[test]
fn script_button_action_fences_auto_run_until_a_modal_or_the_bound() {
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
        .start_test_script(Box::new(QueueBankButton), None)
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
    assert!(
        run_policy.should_defer_auto_run(false),
        "the interaction fences the host's press until the page or the bound"
    );
    assert!(
        !run_policy.should_defer_auto_run(true),
        "an observed modal ends the fence; the modal check holds the press"
    );
    assert!(!run_policy.should_defer_auto_run(false));
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

#[test]
fn native_script_button_action_fences_auto_run_until_a_modal_or_the_bound() {
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
        .start_test_script(Box::new(QueueNativeButton { handle: None }), None)
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

    assert!(
        client.out.pos > 0,
        "the native script sends its interface action"
    );
    assert_eq!(
        client.out.data()[0],
        client::io::ClientProt::IF_BUTTON.id as u8
    );
    assert!(
        run_policy.should_defer_auto_run(false),
        "the interaction fences the host's press until the page or the bound"
    );
    assert!(
        !run_policy.should_defer_auto_run(true),
        "an observed modal ends the fence; the modal check holds the press"
    );
    assert!(!run_policy.should_defer_auto_run(false));
}

/// The native-ledger click is the journal machine's own path
/// (`script_observe.rs`, the ledger arm of `note_script_interaction`): the
/// script begins a machine that emits one `IfButton`, when armed.
struct ClickWhenArmed {
    armed: Arc<std::sync::atomic::AtomicBool>,
    handle: Option<script::native::ActionHandle<NativeButtonMachine>>,
}

impl script::native::Script for ClickWhenArmed {
    fn tick(
        &mut self,
        ctx: &mut script::native::NativeTick<'_>,
    ) -> Result<script::native::ScriptFlow, script::native::ScriptFailure> {
        if self.handle.is_none() && self.armed.swap(false, Ordering::Relaxed) {
            self.handle = Some(
                ctx.actions
                    .begin::<NativeButtonMachine>((), &mut ctx.cx)
                    .expect("native action starts"),
            );
        }
        Ok(script::native::ScriptFlow::Continue)
    }
}

/// One host frame: `drain` PLAYER_INFO packets arrive with `modal` as the
/// client's main modal, and the script clicks on this frame when `click`.
#[derive(Clone, Copy)]
struct Frame {
    drain: u32,
    modal: i32,
    click: bool,
}

const JOURNAL_ROOT: i32 = 8134;

fn frame(drain: u32, modal: i32) -> Frame {
    Frame {
        drain,
        modal,
        click: false,
    }
}

fn click_frame(drain: u32, modal: i32) -> Frame {
    Frame {
        drain,
        modal,
        click: true,
    }
}

/// The production pairing: `Host::run_client` owns the host snapshot and the
/// auto-run decision, and `script_observe_cached` gets host-play's own
/// `script_tick`, one increment per drain with PLAYER_INFO that survives
/// relogs (`play_slots.rs`, the slot thread's `script_tick`).
struct FenceRig {
    scripts: ScriptWall,
    cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    navs: EmptyNav,
    names: Arc<api::obj_names::ObjNames>,
    snapshot: Arc<GameSnapshot>,
    client: Client,
    armed: Arc<std::sync::atomic::AtomicBool>,
    script_tick: Arc<std::sync::atomic::AtomicU64>,
    run_policy: Option<host::ScriptRunPolicy>,
}

impl FenceRig {
    fn new() -> Self {
        let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
        let cheats = Arc::new(Mutex::new(HashMap::new()));
        let mut client = bank_fetch_client();
        // The script's own view is the fixture's: the click's component (601)
        // is visible only under the fixture's bank. The host reads the
        // client's modal roots, which the rig then clears and drives itself.
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(&client);
        client.main_modal_id = -1;
        client.chat_modal_id = -1;
        client.side_modal_id = -1;
        client.runenergy = 20;
        let names = Arc::new(api::obj_names::ObjNames::from_objs(&client.cache.objs));
        let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        script_slot_or_insert(&scripts, "alice")
            .lock()
            .unwrap()
            .start_test_script(
                Box::new(ClickWhenArmed {
                    armed: Arc::clone(&armed),
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
        let tick_edge = Arc::new(std::sync::atomic::AtomicBool::new(false));

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

        let probe_plan = Arc::clone(&plan);
        let probe_index = Arc::clone(&index);
        let probe_edge = Arc::clone(&tick_edge);
        let probe_armed = Arc::clone(&self.armed);

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
                    Some(run_policy),
                );
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
        seen[1..].to_vec()
    }

    /// A relog: the same client and policy, a new session and a new loop. The
    /// client starts the new session's packet counts from its own watermark
    /// (`session_start_gens`, zero here), so the counts restart.
    fn relog(&mut self) {
        self.client.gens = client::client::ClientGens {
            session: self.client.gens.session.wrapping_add(1),
            ..client::client::ClientGens::default()
        };
        self.client.main_modal_id = -1;
    }
}

#[test]
fn auto_run_press_waits_for_the_journal_page_and_its_close() {
    let mut rig = FenceRig::new();
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
        "no press while the click's result is unobserved or the page is up; one after the close"
    );
}

#[test]
fn auto_run_press_waits_for_a_page_that_arrives_a_cycle_late() {
    let mut rig = FenceRig::new();
    let presses = rig.run(&[
        click_frame(1, -1),
        frame(1, -1),
        frame(1, JOURNAL_ROOT),
        frame(1, JOURNAL_ROOT),
        frame(1, -1),
    ]);
    assert_eq!(
        presses,
        [0, 0, 0, 0, 1],
        "the second observed tick has no page yet, and the press still waits for it"
    );
}

#[test]
fn auto_run_press_goes_out_after_the_bound_when_no_page_opens() {
    let mut rig = FenceRig::new();
    let presses = rig.run(&[click_frame(1, -1), frame(1, -1), frame(1, -1), frame(1, -1)]);
    assert_eq!(
        presses,
        [0, 0, 1, 1],
        "two observed ticks without a page end the fence"
    );
}

#[test]
fn auto_run_fence_counts_each_packet_of_a_multi_packet_drain() {
    let mut rig = FenceRig::new();
    let presses = rig.run(&[
        click_frame(1, -1),
        frame(2, JOURNAL_ROOT),
        frame(1, -1),
        frame(1, -1),
    ]);
    assert_eq!(
        presses,
        [0, 0, 1, 1],
        "the page was in the two-packet drain"
    );

    let mut rig = FenceRig::new();
    let presses = rig.run(&[click_frame(1, -1), frame(2, -1), frame(1, -1)]);
    assert_eq!(
        presses,
        [0, 1, 1],
        "two packets are two observed ticks: the bound is spent in one drain"
    );
}

#[test]
fn auto_run_fence_uses_the_same_clock_after_a_relog() {
    let mut rig = FenceRig::new();
    // A long first session: host-play's `script_tick` is now far ahead of
    // the next session's snapshot tick.
    let first = rig.run(&[frame(1, -1); 9]);
    assert_eq!(first[0], 1, "the first session presses once");
    rig.relog();
    let presses = rig.run(&[click_frame(1, -1), frame(1, -1), frame(1, -1), frame(1, -1)]);
    assert_eq!(
        presses,
        [0, 0, 1, 1],
        "the new session's fence lasts two of its own observed ticks"
    );
    assert!(
        rig.script_tick.load(Ordering::Relaxed) >= 13,
        "host-play's counter was carried across the relog, as in production"
    );
}
