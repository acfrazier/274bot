//! Bounded Baker stall mapping: posted pins, filtered loc id, honest results.

use script::isolate_fb::{
    ItemRowInput, ReachViewInput, SceneEntityInput, SnapshotInput, TileInput,
};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};

fn post_snapshot_input(iso: &LoadIsolate, input: &SnapshotInput<'_>) {
    iso.post_snapshot(script::isolate_fb::encode_snapshot(input));
}

fn tile(x: i32, z: i32, level: i32) -> TileInput {
    TileInput { x, z, level }
}

fn base_snapshot<'a>(here: TileInput) -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: Some(here),
        ingame: true,
        inv: &[],
        inv_size: 28,
        stats: &[],
        booths: &[],
        banks: &[],
        bank: &[],
        bank_side: &[],
        bank_open: false,
        bank_loaded: false,
        bank_generation: 0,
        count_dialog_open: false,
        withdraw_x_result_seq: 0,
        withdraw_x_result: false,
        withdraw_load_result_seq: 0,
        withdraw_load_result: false,
        bank_op_result_seq: 0,
        bank_op_result: false,
        hold: false,
        ours: false,
        npcs: &[],
        locs: &[],
        players: &[],
        ground: &[],
        equipment: &[],
        chat_open: false,
        chat_continue: false,
        chat_text: None,
        chat_options: &[],
        side_tab: 0,
        varps: &[],
        combat_styles: &[],
        run_energy: 0,
        run_enabled: false,
        retaliate_enabled: false,
        my_name: Some("bot"),
        in_combat: false,
        animating: false,
        main_modal_id: -1,
        chat_modal_id: -1,
        make_products: &[],
        side_tab_ifaces: &[],
        spell_buttons: &[],
        chat_lines: &[],
        nearest_booth: None,
        bank_note_on: -1,
        bank_note_off: -1,
        scene_state: 2,
        weight: 0,
        camera_yaw: 0,
        camera_pitch: 0,
        teleports_enabled: false,
        self_slot: 0,
        trade_offer_open: false,
        trade_confirm_open: false,
        trade_partner: None,
        trade_mine: &[],
        trade_theirs: &[],
        trade_side: &[],
        trade_accept_id: -1,
        trade_decline_id: -1,
        shop_open: false,
        shop_stock: &[],
        reach: ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
        widgets: &[],
    }
}

fn loc_row<'a>(
    id: i32,
    name: Option<&'a str>,
    x: i32,
    z: i32,
    distance: i32,
    actions: &'a [String],
) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index: id,
        id,
        name,
        x,
        z,
        level: 0,
        distance,
        health: -1,
        max_health: -1,
        in_combat: false,
        animating: false,
        actions,
        reachable: true,
        reachable_adj: true,
        combat_level: 0,
        target_kind: 0,
        target_index: -1,
    }
}

const STEAL: &str = r#"
import { stealCakes, carriedCakes, needsCakeRestock } from '../../api/thieving/CakeStall.js';
import { STALL_TILE, STAND, STAND_ALT, FLEE_TILE, STALL_NAME, STALL_OP, CAKE_ITEMS, classifySteal } from '../../api/thieving/cakeStallData.js';

globalThis.__stolen = 0;
globalThis.__reset = 0;
globalThis.__result = null;
globalThis.__pins = {
    stall: STALL_TILE ? [STALL_TILE.x, STALL_TILE.z, STALL_TILE.level] : null,
    stand: STAND ? [STAND.x, STAND.z, STAND.level] : null,
    alt: STAND_ALT ? [STAND_ALT.x, STAND_ALT.z, STAND_ALT.level] : null,
    flee: FLEE_TILE ? [FLEE_TILE.x, FLEE_TILE.z, FLEE_TILE.level] : null,
    name: STALL_NAME,
    op: STALL_OP,
    items: CAKE_ITEMS.slice(),
};
try {
    classifySteal();
    globalThis.__classify = 'called';
} catch (e) {
    globalThis.__classify = String(e && e.message ? e.message : e);
}

export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__clearFacts) {
            const content = globalThis.__rs2b0t_host.content || {};
            delete content.baker_stall;
            globalThis.__rs2b0t_host.content = content;
        }
        globalThis.__carried = carriedCakes();
        globalThis.__need = needsCakeRestock(globalThis.__needTarget);
        globalThis.__result = await stealCakes({
            fillTo: globalThis.__fillTo,
            abort: () => globalThis.__abort === true,
            shouldEat: () => globalThis.__eat === true,
            lockedOutUntil: () => globalThis.__lockUntil ?? 0,
            setStatus: () => {},
            log: () => {},
            onSteal: () => { globalThis.__stolen += 1; },
            onReset: () => { globalThis.__reset += 1; },
        });
    }
}
"#;

fn spawn() -> LoadIsolate {
    LoadIsolate::spawn(STEAL.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn tick(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    let _ = iso.probe("true");
}

fn steal_loc() -> InteractReq {
    InteractReq::Loc {
        x: 2667,
        z: 3310,
        level: 0,
        action: "Steal from".into(),
        id: Some(2561),
    }
}

fn wait_result(iso: &LoadIsolate) -> serde_json::Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let _ = iso.probe("true");
        let value = iso.probe("globalThis.__result").unwrap();
        if !value.is_null() || std::time::Instant::now() > deadline {
            return value;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn posted_pins_match_selected_274_289_stall_and_live_stand() {
    let iso = spawn();
    let pins = iso.probe("__pins").unwrap();
    assert_eq!(pins["stall"], serde_json::json!([2667, 3310, 0]));
    assert_eq!(pins["stand"], serde_json::json!([2668, 3312, 0]));
    assert_eq!(pins["alt"], serde_json::json!([2669, 3310, 0]));
    assert_eq!(pins["flee"], serde_json::json!([2655, 3298, 0]));
    assert_eq!(pins["name"], "Baker's stall");
    assert_eq!(pins["op"], "Steal from");
    assert_eq!(
        pins["items"],
        serde_json::json!(["Cake", "Bread", "Chocolate slice"])
    );
    assert_eq!(
        iso.probe("__classify").unwrap(),
        "not impl: cakeStallData.classifySteal"
    );
    iso.join();
}

#[test]
fn closer_door_still_selects_qualifying_stall_id() {
    let iso = spawn();
    iso.probe("globalThis.__fillTo = 28").unwrap();
    let open = ["Open".to_string()];
    let steal = ["Steal from".to_string()];
    let locs = [
        loc_row(1530, Some("Door"), 2668, 3312, 0, &open),
        loc_row(2561, Some("Baker's stall"), 2667, 3310, 2, &steal),
    ];
    let mut snap = base_snapshot(tile(2668, 3312, 0));
    snap.locs = &locs;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![steal_loc()]);
    assert!(iso.probe("globalThis.__result").unwrap().is_null());
    iso.probe("globalThis.__abort = true").unwrap();
    tick(&iso, 2);
    assert_eq!(wait_result(&iso), "aborted");
    assert_eq!(iso.probe("__stolen").unwrap(), 0);
    assert_eq!(iso.probe("__reset").unwrap(), 0);
    iso.join();
}

#[test]
fn other_stall_depleted_wrong_op_and_missing_facts_queue_nothing() {
    let steal = ["Steal from".to_string()];
    let examine = ["Examine".to_string()];
    let open = ["Open".to_string()];

    let iso = spawn();
    iso.probe("globalThis.__fillTo = 28").unwrap();
    let locs = [
        loc_row(1530, Some("Door"), 2656, 3311, 0, &open),
        loc_row(2561, Some("Baker's stall"), 2655, 3311, 1, &steal),
        loc_row(2561, Some("Baker's stall"), 2667, 3310, 12, &steal),
    ];
    let mut snap = base_snapshot(tile(2656, 3311, 0));
    snap.locs = &locs;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::WalkTo {
            x: 2668,
            z: 3312,
            level: 0
        }]
    );
    snap.here = Some(tile(2668, 3312, 0));
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.drain_interacts(), vec![steal_loc()]);
    iso.probe("globalThis.__abort = true").unwrap();
    tick(&iso, 3);
    assert_eq!(wait_result(&iso), "aborted");
    iso.join();

    let iso = spawn();
    iso.probe("globalThis.__fillTo = 28").unwrap();
    let locs = [
        loc_row(2561, Some("Baker's stall"), 2667, 3310, 1, &examine),
        loc_row(2561, Some("Bakery stall"), 2667, 3310, 1, &steal),
        loc_row(99, Some("Baker's stall"), 2667, 3310, 1, &steal),
    ];
    let mut snap = base_snapshot(tile(2668, 3312, 0));
    snap.locs = &locs;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert!(iso.drain_interacts().is_empty());
    assert_eq!(wait_result(&iso), "no-progress");
    iso.join();

    let iso = spawn();
    iso.probe("globalThis.__clearFacts = true").unwrap();
    iso.probe("globalThis.__fillTo = 28").unwrap();
    let locs = [loc_row(2561, Some("Baker's stall"), 2667, 3310, 1, &steal)];
    let mut snap = base_snapshot(tile(2668, 3312, 0));
    snap.locs = &locs;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert!(iso.drain_interacts().is_empty());
    assert_eq!(wait_result(&iso), "no-progress");
    iso.join();
}

#[test]
fn gates_skip_dispatch_and_walk_revalidates() {
    let steal = ["Steal from".to_string()];
    let locs = [loc_row(2561, Some("Baker's stall"), 2667, 3310, 1, &steal)];

    let iso = spawn();
    iso.probe("globalThis.__abort = true").unwrap();
    let mut snap = base_snapshot(tile(2668, 3312, 0));
    snap.locs = &locs;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert!(iso.drain_interacts().is_empty());
    assert_eq!(wait_result(&iso), "aborted");
    iso.join();

    let iso = spawn();
    iso.probe("globalThis.__eat = true").unwrap();
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert!(iso.drain_interacts().is_empty());
    assert_eq!(wait_result(&iso), "aborted");
    iso.join();

    let iso = spawn();
    snap.in_combat = true;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert!(iso.drain_interacts().is_empty());
    assert_eq!(wait_result(&iso), "combat");
    snap.in_combat = false;
    iso.join();

    let iso = spawn();
    iso.probe("globalThis.__lockUntil = 10").unwrap();
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert!(iso.drain_interacts().is_empty());
    assert_eq!(wait_result(&iso), "no-progress");
    iso.join();

    let cake = [ItemRowInput::nc(Some("Cake"), 1)];
    let iso = spawn();
    iso.probe("globalThis.__fillTo = 1").unwrap();
    snap.inv = &cake;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert!(iso.drain_interacts().is_empty());
    assert_eq!(wait_result(&iso), "stocked");
    snap.inv = &[];
    iso.join();

    let full: Vec<ItemRowInput> = (0..28)
        .map(|_| ItemRowInput::nc(Some("Rune scimitar"), 1))
        .collect();
    let iso = spawn();
    iso.probe("globalThis.__fillTo = 28").unwrap();
    snap.inv = &full;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert!(iso.drain_interacts().is_empty());
    assert_eq!(wait_result(&iso), "stocked");
    assert_eq!(iso.probe("__need").unwrap(), false);
    iso.join();

    let iso = spawn();
    iso.probe("globalThis.__fillTo = 28").unwrap();
    snap.inv = &[];
    snap.here = Some(tile(2661, 3306, 0));
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::WalkTo {
            x: 2668,
            z: 3312,
            level: 0
        }]
    );
    iso.probe("delete globalThis.__rs2b0t_host.content.baker_stall")
        .unwrap();
    snap.here = Some(tile(2668, 3312, 0));
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert!(iso.drain_interacts().is_empty());
    assert_eq!(wait_result(&iso), "no-progress");
    iso.join();
}

#[test]
fn food_delta_calls_onsteal_once_and_partial_is_not_stocked() {
    let steal = ["Steal from".to_string()];
    let locs = [loc_row(2561, Some("Baker's stall"), 2667, 3310, 1, &steal)];
    let cake = [ItemRowInput::nc(Some("Cake"), 1)];
    let chocolate = [ItemRowInput::nc(Some("Chocolate cake"), 1)];

    let iso = spawn();
    iso.probe("globalThis.__fillTo = 28").unwrap();
    let mut snap = base_snapshot(tile(2668, 3312, 0));
    snap.locs = &locs;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![steal_loc()]);
    assert_eq!(iso.probe("__stolen").unwrap(), 0);
    snap.inv = &cake;
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(wait_result(&iso), "no-progress");
    assert_eq!(iso.probe("__stolen").unwrap(), 1);
    assert_eq!(iso.probe("__reset").unwrap(), 0);
    assert_eq!(iso.probe("__carried").unwrap(), 0);
    iso.join();

    let iso = spawn();
    iso.probe("globalThis.__fillTo = 1").unwrap();
    snap.inv = &[];
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![steal_loc()]);
    snap.inv = &cake;
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(wait_result(&iso), "stocked");
    assert_eq!(iso.probe("__stolen").unwrap(), 1);
    iso.join();

    let iso = spawn();
    iso.probe("globalThis.__fillTo = 28").unwrap();
    snap.inv = &chocolate;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![steal_loc()]);
    assert_eq!(iso.probe("__stolen").unwrap(), 0);
    iso.probe("globalThis.__abort = true").unwrap();
    tick(&iso, 2);
    assert_eq!(wait_result(&iso), "aborted");
    assert_eq!(iso.probe("__stolen").unwrap(), 0);
    assert_eq!(iso.probe("__need").unwrap(), true);
    iso.join();
}

#[test]
fn result_is_never_boolean_and_classify_is_not_on_the_path() {
    let iso = spawn();
    iso.probe("globalThis.__fillTo = 28").unwrap();
    let snap = base_snapshot(tile(2668, 3312, 0));
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    let result = wait_result(&iso);
    assert_eq!(result, "no-progress");
    assert!(result.as_str().is_some());
    assert_ne!(result, true);
    assert_ne!(result, false);
    assert_eq!(
        iso.probe("__classify").unwrap(),
        "not impl: cakeStallData.classifySteal"
    );
    iso.join();
}
