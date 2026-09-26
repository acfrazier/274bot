//! Bounded Baker stall mapping: posted pins, filtered loc id, honest results.

use script::isolate_fb::{
    ChatLineInput, IsolateBuf, ItemRowInput, ReachViewInput, SceneEntityInput, SnapshotFingerprint,
    SnapshotInput, TileInput,
};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};

mod common;
use common::post_snapshot_input;

fn post_snapshot_delta(
    iso: &LoadIsolate,
    encoder: &mut IsolateBuf,
    last: &mut Option<SnapshotFingerprint>,
    input: &SnapshotInput<'_>,
) {
    let (bytes, next) = encoder.encode_snapshot_delta(last.as_ref(), input, false);
    *last = Some(next);
    iso.post_snapshot(bytes);
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
        combat_level: 0,
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
        self_target_kind: 0,
        self_target_index: -1,
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
        size: 0,
        nx: 0,
        nz: 0,
        shape: 0,
        angle: 0,
    }
}

const STEAL: &str = r#"
import { stealCakes, carriedCakes, needsCakeRestock } from '../../api/thieving/CakeStall.js';
import { STALL_TILE, STAND, STAND_ALT, FLEE_TILE, STALL_NAME, STALL_OP, CAKE_ITEMS, classifySteal } from '../../api/thieving/cakeStallData.js';

globalThis.__stolen = 0;
globalThis.__reset = 0;
globalThis.__receivers = [];
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
        const opts = {
            fillTo: globalThis.__fillTo,
            abort() {
                globalThis.__receivers.push(['abort', this === opts]);
                return globalThis.__abort === true;
            },
            shouldEat() {
                globalThis.__receivers.push(['shouldEat', this === opts]);
                return globalThis.__eat === true;
            },
            lockedOutUntil() {
                globalThis.__receivers.push(['lockedOutUntil', this === opts]);
                return globalThis.__lockUntil ?? 0;
            },
            setStatus(s) {
                globalThis.__receivers.push(['setStatus', this === opts]);
                (globalThis.__events ||= []).push(['status', s]);
            },
            log(m) {
                globalThis.__receivers.push(['log', this === opts]);
                (globalThis.__logs ||= []).push(m);
                (globalThis.__events ||= []).push(['log', m]);
            },
            onSteal() {
                globalThis.__receivers.push(['onSteal', this === opts]);
                globalThis.__stolen += 1;
            },
            onReset() {
                globalThis.__receivers.push(['onReset', this === opts]);
                globalThis.__reset += 1;
                (globalThis.__events ||= []).push(['reset']);
            },
        };
        globalThis.__cakeOpts = opts;
        globalThis.__result = await stealCakes(opts);
        globalThis.__returns = (globalThis.__returns ?? 0) + 1;
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

/// Game ticks settle parked waits, so keep ticking (100 ms apart, past the
/// 2.4 s steal resolve window) until the call returns.
fn wait_result(iso: &LoadIsolate) -> serde_json::Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut n = 100;
    loop {
        tick(iso, n);
        n += 1;
        let value = iso.probe("globalThis.__result").unwrap();
        if !value.is_null() || std::time::Instant::now() > deadline {
            return value;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
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
fn native_cake_stall_does_not_roundtrip_js_snapshot_collections() {
    let iso = spawn();
    iso.probe("globalThis.__fillTo = 28").unwrap();
    let steal = ["Steal from".to_string()];
    let locs = [loc_row(2561, Some("Baker's stall"), 2667, 3310, 2, &steal)];
    let mut snap = base_snapshot(tile(2668, 3312, 0));
    snap.locs = &locs;
    post_snapshot_input(&iso, &snap);
    iso.probe(
        r#"(() => {
            Object.defineProperties(globalThis.__rs2b0t_host.snapshot, {
                locs: { get() { throw new Error('locs crossed JS/native seam'); } },
                inv: { get() { throw new Error('inv crossed JS/native seam'); } },
            });
            return true;
        })()"#,
    )
    .unwrap();

    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![steal_loc()]);
    iso.join();
}

#[test]
fn hooks_are_resolved_again_at_each_invocation() {
    let steal = ["Steal from".to_string()];
    let locs = [loc_row(2561, Some("Baker's stall"), 2667, 3310, 1, &steal)];
    let cake = [ItemRowInput::nc(Some("Cake"), 1)];
    let iso = spawn();
    iso.probe("globalThis.__fillTo = 28").unwrap();
    let mut encoder = IsolateBuf::new();
    let mut last = None;
    let mut snap = base_snapshot(tile(2668, 3312, 0));
    snap.locs = &locs;
    post_snapshot_delta(&iso, &mut encoder, &mut last, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![steal_loc()]);

    iso.probe(
        r#"(() => {
            globalThis.__dynamicAbort = false;
            globalThis.__dynamicSteals = 0;
            globalThis.__dynamicReceivers = [];
            globalThis.__cakeOpts.abort = function () {
                globalThis.__dynamicReceivers.push(this === globalThis.__cakeOpts);
                return globalThis.__dynamicAbort;
            };
            globalThis.__cakeOpts.onSteal = function () {
                globalThis.__dynamicReceivers.push(this === globalThis.__cakeOpts);
                globalThis.__dynamicSteals += 1;
            };
            return true;
        })()"#,
    )
    .unwrap();

    snap.inv = &cake;
    snap.tick = 2;
    post_snapshot_delta(&iso, &mut encoder, &mut last, &snap);
    tick(&iso, 2);
    tick(&iso, 3);
    assert_eq!(iso.probe("globalThis.__dynamicSteals").unwrap(), 1);

    iso.probe("globalThis.__dynamicAbort = true").unwrap();
    assert_eq!(wait_result(&iso), "aborted");
    let receivers = iso.probe("globalThis.__dynamicReceivers").unwrap();
    assert!(
        receivers
            .as_array()
            .expect("dynamic hook receivers")
            .iter()
            .all(|value| value == true),
        "{receivers:?}"
    );
    iso.join();
}

#[test]
fn rust_selected_facts_outlive_js_content_and_invalid_stalls_queue_nothing() {
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
    tick(&iso, 2);
    // No qualifying stall: the call waits for a restock, sends nothing and
    // returns only when the caller aborts.
    assert!(iso.drain_interacts().is_empty());
    assert!(iso.probe("globalThis.__result").unwrap().is_null());
    let logs = iso.probe("globalThis.__logs").unwrap();
    assert!(
        logs.to_string()
            .contains("stall emptied — waiting for the restock"),
        "{logs}"
    );
    iso.probe("globalThis.__abort = true").unwrap();
    assert_eq!(wait_result(&iso), "aborted");
    assert!(iso.drain_interacts().is_empty());
    iso.join();

    let iso = spawn();
    iso.probe("globalThis.__clearFacts = true").unwrap();
    iso.probe("globalThis.__fillTo = 28").unwrap();
    let locs = [loc_row(2561, Some("Baker's stall"), 2667, 3310, 1, &steal)];
    let mut snap = base_snapshot(tile(2668, 3312, 0));
    snap.locs = &locs;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![steal_loc()]);
    iso.probe("globalThis.__abort = true").unwrap();
    assert_eq!(wait_result(&iso), "aborted");
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
    // A caller lockout holds the steal until its tick; the same call then
    // steals.
    assert!(iso.drain_interacts().is_empty());
    assert!(iso.probe("globalThis.__result").unwrap().is_null());
    snap.tick = 10;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 10);
    assert_eq!(iso.drain_interacts(), vec![steal_loc()]);
    snap.tick = 1;
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
    assert_eq!(iso.drain_interacts(), vec![steal_loc()]);
    iso.probe("globalThis.__abort = true").unwrap();
    assert_eq!(wait_result(&iso), "aborted");
    iso.join();
}

#[test]
fn food_delta_calls_onsteal_once_and_partial_counts_as_stocked() {
    let steal = ["Steal from".to_string()];
    let locs = [loc_row(2561, Some("Baker's stall"), 2667, 3310, 1, &steal)];
    let cake = [ItemRowInput::nc(Some("Cake"), 1)];
    let slice = [ItemRowInput::nc(Some("Slice of cake"), 1)];
    let two_thirds = [ItemRowInput::nc(Some("2/3 cake"), 1)];

    let iso = spawn();
    iso.probe("globalThis.__fillTo = 28").unwrap();
    let mut encoder = IsolateBuf::new();
    let mut last = None;
    let mut snap = base_snapshot(tile(2668, 3312, 0));
    snap.locs = &locs;
    post_snapshot_delta(&iso, &mut encoder, &mut last, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![steal_loc()]);
    assert_eq!(iso.probe("__stolen").unwrap(), 0);
    snap.inv = &cake;
    snap.tick = 2;
    post_snapshot_delta(&iso, &mut encoder, &mut last, &snap);
    tick(&iso, 2);
    tick(&iso, 3);
    // The gain is counted and the same call steals again toward fillTo.
    assert_eq!(iso.probe("__stolen").unwrap(), 1);
    assert_eq!(iso.drain_interacts(), vec![steal_loc()]);
    assert!(iso.probe("globalThis.__result").unwrap().is_null());
    assert_eq!(iso.probe("__reset").unwrap(), 0);
    assert_eq!(iso.probe("__carried").unwrap(), 0);
    iso.probe("globalThis.__abort = true").unwrap();
    assert_eq!(wait_result(&iso), "aborted");
    assert_eq!(iso.probe("__stolen").unwrap(), 1);
    let receivers: Vec<serde_json::Value> =
        serde_json::from_value(iso.probe("globalThis.__receivers").unwrap()).unwrap();
    assert!(
        receivers.contains(&serde_json::json!(["onSteal", true]))
            && receivers.iter().all(|row| row[1] == true),
        "{receivers:?}"
    );
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

    // Frozen foodTarget 1 is already met by a remaining slice or 2/3 cake.
    let iso = spawn();
    iso.probe("globalThis.__fillTo = 1").unwrap();
    iso.probe("globalThis.__needTarget = 1").unwrap();
    snap.inv = &slice;
    snap.tick = 1;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert!(iso.drain_interacts().is_empty());
    assert_eq!(wait_result(&iso), "stocked");
    assert_eq!(iso.probe("__stolen").unwrap(), 0);
    assert_eq!(iso.probe("__carried").unwrap(), 1);
    assert_eq!(iso.probe("__need").unwrap(), false);
    iso.join();

    let iso = spawn();
    iso.probe("globalThis.__fillTo = 1").unwrap();
    iso.probe("globalThis.__needTarget = 1").unwrap();
    snap.inv = &two_thirds;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert!(iso.drain_interacts().is_empty());
    assert_eq!(wait_result(&iso), "stocked");
    assert_eq!(iso.probe("__stolen").unwrap(), 0);
    assert_eq!(iso.probe("__carried").unwrap(), 1);
    assert_eq!(iso.probe("__need").unwrap(), false);
    iso.join();
}

#[test]
fn slice_of_cake_at_food_target_one_does_not_restock() {
    let steal = ["Steal from".to_string()];
    let locs = [loc_row(2561, Some("Baker's stall"), 2667, 3310, 1, &steal)];
    let slice = [ItemRowInput::nc(Some("Slice of cake"), 1)];
    let lobster = [ItemRowInput::nc(Some("Lobster"), 1)];

    let iso = spawn();
    iso.probe("globalThis.__fillTo = 1").unwrap();
    iso.probe("globalThis.__needTarget = 1").unwrap();
    let mut snap = base_snapshot(tile(2668, 3312, 0));
    snap.locs = &locs;
    snap.inv = &slice;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert!(
        iso.drain_interacts().is_empty(),
        "slice satisfies foodTarget 1; no steal"
    );
    assert_eq!(wait_result(&iso), "stocked");
    assert_eq!(iso.probe("__carried").unwrap(), 1);
    assert_eq!(iso.probe("__need").unwrap(), false);
    iso.join();

    // Unrelated food still leaves the stall restock owed.
    let iso = spawn();
    iso.probe("globalThis.__fillTo = 1").unwrap();
    iso.probe("globalThis.__needTarget = 1").unwrap();
    snap.inv = &lobster;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![steal_loc()]);
    assert_eq!(iso.probe("__carried").unwrap(), 0);
    assert_eq!(iso.probe("__need").unwrap(), true);
    iso.probe("globalThis.__abort = true").unwrap();
    assert_eq!(wait_result(&iso), "aborted");
    iso.join();
}

#[test]
fn result_is_never_boolean_and_classify_is_not_on_the_path() {
    let iso = spawn();
    iso.probe("globalThis.__fillTo = 28").unwrap();
    let snap = base_snapshot(tile(2668, 3312, 0));
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    iso.probe("globalThis.__abort = true").unwrap();
    let result = wait_result(&iso);
    assert_eq!(result, "aborted");
    assert!(result.as_str().is_some());
    assert_ne!(result, true);
    assert_ne!(result, false);
    assert_eq!(
        iso.probe("__classify").unwrap(),
        "not impl: cakeStallData.classifySteal"
    );
    iso.join();
}

/// The stall owner watching the stand refuses steals silently: nothing is
/// gained and no guard comes. After three, the frozen `stealCakes` swaps to
/// the other stand and reports it through `log` and `onReset`.
#[test]
fn watched_stand_swaps_after_three_refused_steals() {
    let steal = ["Steal from".to_string()];
    let locs = [loc_row(2561, Some("Baker's stall"), 2667, 3310, 1, &steal)];
    let iso = spawn();
    iso.probe("globalThis.__fillTo = 28").unwrap();
    let mut snap = base_snapshot(tile(2668, 3312, 0));
    snap.locs = &locs;
    post_snapshot_input(&iso, &snap);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut steals = 0;
    let mut walked = None;
    let mut n = 1;
    while walked.is_none() && std::time::Instant::now() < deadline {
        tick(&iso, n);
        n += 1;
        for req in iso.drain_interacts() {
            match req {
                InteractReq::Loc { .. } => steals += 1,
                InteractReq::WalkTo { x, z, level } => walked = Some((x, z, level)),
                _ => {}
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(steals, 3, "three refused steals from the watched stand");
    assert_eq!(walked, Some((2669, 3310, 0)), "then the alternate stand");
    assert_eq!(iso.probe("__reset").unwrap(), 1);
    let logs = iso.probe("globalThis.__logs").unwrap();
    assert!(
        logs.to_string()
            .contains("3 refused steals — swapping to the stand at (2669,3310)"),
        "{logs}"
    );
    // The pump hands the caller each step's status, then its log, then
    // onReset, in the frozen order (ArdyCakes paints the status).
    let events: Vec<serde_json::Value> =
        serde_json::from_value(iso.probe("globalThis.__events").unwrap()).unwrap();
    assert!(
        events.contains(&serde_json::json!(["status", "stealing cake (0/28)"])),
        "{events:?}"
    );
    let swap = events
        .iter()
        .position(|event| *event == serde_json::json!(["status", "watched — swapping stands"]))
        .unwrap_or_else(|| panic!("no swap status: {events:?}"));
    assert_eq!(
        events[swap + 1..swap + 3],
        [
            serde_json::json!([
                "log",
                "3 refused steals — swapping to the stand at (2669,3310)"
            ]),
            serde_json::json!(["reset"]),
        ],
        "{events:?}"
    );
    let receivers: Vec<serde_json::Value> =
        serde_json::from_value(iso.probe("globalThis.__receivers").unwrap()).unwrap();
    for name in ["setStatus", "log", "onReset"] {
        assert!(
            receivers.contains(&serde_json::json!([name, true])),
            "{name}: {receivers:?}"
        );
    }
    assert!(receivers.iter().all(|row| row[1] == true), "{receivers:?}");
    iso.join();
}

/// Within ten ticks of combat the server refuses a stall steal with a chat
/// line. The frozen `stealCakes` waits that lockout out before the next steal
/// and does not count it as the owner watching the stand.
#[test]
fn combat_lockout_line_holds_the_next_steal_for_ten_ticks() {
    let steal = ["Steal from".to_string()];
    let locs = [loc_row(2561, Some("Baker's stall"), 2667, 3310, 1, &steal)];
    let lockout = [ChatLineInput {
        seq: 5,
        text: "You can't steal from the market stall during combat!",
        type_: 0,
        username: None,
    }];
    let iso = spawn();
    iso.probe("globalThis.__fillTo = 28").unwrap();
    let mut snap = base_snapshot(tile(2668, 3312, 0));
    snap.locs = &locs;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![steal_loc()]);

    // The line lands on tick 2. Ticks 3..=11 span more than the 2.4 s
    // resolve window, so a steal that ignored the line would retry in here.
    snap.chat_lines = &lockout;
    for n in 2..12 {
        snap.tick = n;
        post_snapshot_input(&iso, &snap);
        tick(&iso, n);
        assert!(iso.drain_interacts().is_empty(), "tick {n}");
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    snap.tick = 12;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 12);
    assert_eq!(iso.drain_interacts(), vec![steal_loc()], "same stand");
    assert_eq!(iso.probe("__reset").unwrap(), 0);
    iso.join();
}

/// Frozen `stealCakes` is one loop: a gain goes back to its head and the
/// same call steals again until `fillTo`. Callers such as ArdyThiever's
/// `RestockCakes` rely on one call filling to their food target.
#[test]
fn one_call_steals_until_fill_to_before_returning() {
    let steal = ["Steal from".to_string()];
    let locs = [loc_row(2561, Some("Baker's stall"), 2667, 3310, 1, &steal)];
    let one = [ItemRowInput::nc(Some("Cake"), 1)];
    let two = [ItemRowInput::nc(Some("Cake"), 2)];
    let iso = spawn();
    iso.probe("globalThis.__fillTo = 2").unwrap();
    let mut snap = base_snapshot(tile(2668, 3312, 0));
    snap.locs = &locs;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![steal_loc()]);

    snap.inv = &one;
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.drain_interacts(), vec![steal_loc()], "second steal");
    assert_eq!(iso.probe("__stolen").unwrap(), 1);
    assert!(
        iso.probe("globalThis.__returns").unwrap().is_null(),
        "the first gain must not return to the caller"
    );

    snap.inv = &two;
    snap.tick = 3;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 3);
    // Returned once, on this tick, with the pack filled.
    assert_eq!(iso.probe("globalThis.__result").unwrap(), "stocked");
    assert_eq!(iso.probe("__stolen").unwrap(), 2);
    assert_eq!(iso.probe("globalThis.__returns").unwrap(), 1);
    let logs = iso.probe("globalThis.__logs").unwrap();
    assert!(
        logs.to_string().contains("stocked 2 stall food (1 slots)"),
        "{logs}"
    );
    iso.join();
}
