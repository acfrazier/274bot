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
fn script_button_action_defers_auto_run_until_the_next_observed_tick() {
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
    assert!(run_policy.should_defer_auto_run(1));
    assert!(
        !run_policy.should_defer_auto_run(2),
        "the host can act after the next observed game tick"
    );
}
