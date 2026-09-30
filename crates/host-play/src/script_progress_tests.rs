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
            script::LoadShape::CompatClass,
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
