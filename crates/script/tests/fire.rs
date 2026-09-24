// Task 40 — `lightFire` and next-tile composed end to end: the shim marshals
// the log name / plot / refused keys, Rust owns the start/light waits and
// AABB selection, and a queued use-on is not a lit fire.

use script::isolate_fb::{
    ChatLineInput, ItemRowInput, ReachViewInput, SceneEntityInput, SnapshotInput, StatInput,
    TileInput,
};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};
use serde_json::Value;

fn spawn(src: &str) -> LoadIsolate {
    LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn row<'a>(name: &'a str, id: i32, count: i32, slot: i32) -> ItemRowInput<'a> {
    ItemRowInput {
        name: Some(name),
        count,
        id,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: -1,
        slot,
    }
}

fn fm(xp: i32) -> StatInput<'static> {
    StatInput {
        index: 11,
        name: "firemaking",
        xp,
        base: 1,
        effective: 1,
    }
}

fn loc<'a>(name: &'a str, x: i32, z: i32) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index: 0,
        id: 1,
        name: Some(name),
        x,
        z,
        level: 0,
        distance: 0,
        health: -1,
        max_health: -1,
        in_combat: false,
        animating: false,
        actions: &[],
        reachable: true,
        reachable_adj: true,
        combat_level: 0,
        target_kind: 0,
        target_index: -1,
        size: 0,
        nx: 0,
        nz: 0,
    }
}

fn walkable_words(
    width: i32,
    height: i32,
    base_x: i32,
    base_z: i32,
    tiles: &[(i32, i32)],
) -> Vec<u32> {
    let n = (width as usize).saturating_mul(height as usize);
    let mut words = vec![0u32; n.div_ceil(32)];
    for &(x, z) in tiles {
        let lx = x - base_x;
        let lz = z - base_z;
        if lx < 0 || lz < 0 || lx >= width || lz >= height {
            continue;
        }
        let i = (lx as usize) * (height as usize) + (lz as usize);
        words[i / 32] |= 1u32 << (i % 32);
    }
    words
}

fn base<'a>() -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: Some(TileInput {
            x: 3235,
            z: 3418,
            level: 0,
        }),
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

fn post(iso: &LoadIsolate, snap: &SnapshotInput<'_>) {
    iso.post_snapshot(script::isolate_fb::encode_snapshot(snap));
}

fn tick(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    let _ = iso.probe("true");
}

const LIGHT: &str = r#"
import { lightFire } from '../../api/firemaking/LightFire.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        try {
            globalThis.__ok = await lightFire(globalThis.__log || 'Logs');
        } catch (e) {
            globalThis.__ok = String(e.message || e);
        }
    }
}
"#;

const LANE: &str = r#"
import { findBurnLane, inFirePlot, NoLightTiles, tileKey, burnLaneWant, fireReactionTicks, isBurnWest } from '../../api/firemaking/Firemaking.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        try {
            const plot = globalThis.__plot;
            const occupied = new Set(globalThis.__occupied || []);
            globalThis.__found = findBurnLane(plot, globalThis.__here, occupied, 3, undefined, undefined, [{dx: -1, dz: 0}]);
            globalThis.__in = inFirePlot(globalThis.__here, plot);
            globalThis.__want = burnLaneWant(20);
            globalThis.__ticks = fireReactionTicks();
            globalThis.__west = isBurnWest({ dx: -1, dz: 0 });
            const no = new NoLightTiles();
            no.add({ x: 1, z: 2 });
            globalThis.__merged = [...no.merge(new Set(['2,3', '1,2']))];
            no.clear();
            globalThis.__refused = no.has({ x: 1, z: 2 }) === false && no.size === 0 && tileKey({ x: 1, z: 2 }) === '1,2';
            globalThis.__ok = true;
        } catch (e) {
            globalThis.__ok = String(e.message || e);
        }
    }
}
"#;

const RESTOCK: &str = r#"
import { exactTool, toolRestockPlan, TINDERBOX, bestPickaxe } from '../../api/acquisition/Tools.js';
export default class T extends LoopingBot {
    loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        try {
            const inv = globalThis.__inv || {};
            const bank = globalThis.__bank || {};
            globalThis.__plan = toolRestockPlan(
                [exactTool(TINDERBOX)],
                () => 1,
                (n) => inv[n] || 0,
                (n) => bank[n] || 0,
            );
            globalThis.__hammer = toolRestockPlan([exactTool('Hammer')], () => 1, () => 0, () => 1);
            globalThis.__pick = bestPickaxe(99, () => false);
            globalThis.__ok = true;
        } catch (e) {
            globalThis.__ok = String(e.message || e);
        }
    }
}
"#;

fn tinder_use_on() -> InteractReq {
    InteractReq::UseOn {
        name: "Tinderbox".into(),
        kind: "inv".into(),
        target_name: Some("Logs".into()),
        x: 0,
        z: 0,
        level: 0,
        index: None,
        source_item_id: Some(590),
        source_item_slot: Some(0),
        target_item_id: Some(1511),
        target_item_slot: Some(1),
    }
}

#[test]
fn light_fire_queues_tinderbox_use_on_and_is_not_lit_yet() {
    let iso = spawn(LIGHT);
    let inv = [row("Tinderbox", 590, 1, 0), row("Logs", 1511, 5, 1)];
    let stats = [fm(0)];
    let mut snap = base();
    snap.inv = &inv;
    snap.stats = &stats;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![tinder_use_on()],
        "lightFire is tinderbox use-on logs"
    );
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Null,
        "a queued use-on is not a completed light"
    );
    iso.join();
}

#[test]
fn light_fire_xp_is_lit() {
    let iso = spawn(LIGHT);
    let inv = [row("Tinderbox", 590, 1, 0), row("Logs", 1511, 5, 1)];
    let stats = [fm(0)];
    let mut snap = base();
    snap.inv = &inv;
    snap.stats = &stats;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts().len(), 1);

    let inv2 = [row("Tinderbox", 590, 1, 0), row("Logs", 1511, 4, 1)];
    let stats2 = [fm(40)];
    snap.tick = 2;
    snap.inv = &inv2;
    snap.stats = &stats2;
    snap.animating = true;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.probe("__ok").unwrap(), "lit");
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn cant_light_is_blocked() {
    let iso = spawn(LIGHT);
    let inv = [row("Tinderbox", 590, 1, 0), row("Logs", 1511, 5, 1)];
    let stats = [fm(0)];
    let mut snap = base();
    snap.inv = &inv;
    snap.stats = &stats;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts().len(), 1);

    let lines = [ChatLineInput {
        seq: 4,
        text: "You can't light a fire here.",
        type_: 0,
        username: None,
    }];
    snap.tick = 2;
    snap.chat_lines = &lines;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.probe("__ok").unwrap(), "blocked");
    iso.join();
}

#[test]
fn start_timeout_without_xp_or_chat_is_stalled() {
    let iso = spawn(LIGHT);
    let inv = [row("Tinderbox", 590, 1, 0), row("Logs", 1511, 5, 1)];
    let stats = [fm(0)];
    let mut snap = base();
    snap.inv = &inv;
    snap.stats = &stats;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts().len(), 1);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);

    for n in 2..=15 {
        snap.tick = n;
        post(&iso, &snap);
        tick(&iso, n);
    }
    assert_eq!(iso.probe("__ok").unwrap(), "stalled");
    assert!(
        iso.drain_interacts().is_empty(),
        "timeout must not send again"
    );
    iso.join();
}

#[test]
fn missing_tinderbox_or_logs_stalls_without_a_packet() {
    let iso = spawn(LIGHT);
    let inv = [row("Logs", 1511, 5, 1)];
    let mut snap = base();
    snap.inv = &inv;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), "stalled");
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn pause_hold_and_session_reset_abort_a_parked_light() {
    let iso = spawn(LIGHT);
    let inv = [row("Tinderbox", 590, 1, 0), row("Logs", 1511, 5, 1)];
    let stats = [fm(0)];
    let mut snap = base();
    snap.inv = &inv;
    snap.stats = &stats;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts().len(), 1);

    iso.pause();
    snap.tick = 2;
    snap.animating = true;
    let stats2 = [fm(40)];
    snap.stats = &stats2;
    post(&iso, &snap);
    tick(&iso, 2);
    tick(&iso, 3);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Null,
        "Pause must not complete a parked light"
    );
    iso.resume();

    snap.hold = true;
    snap.tick = 4;
    post(&iso, &snap);
    tick(&iso, 4);
    tick(&iso, 5);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Null,
        "Guardian hold must freeze the parked light wait"
    );

    iso.reset_session_work();
    snap.hold = false;
    snap.tick = 6;
    post(&iso, &snap);
    tick(&iso, 6);
    tick(&iso, 7);
    assert_eq!(iso.probe("__ok").unwrap(), "stalled");
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn find_burn_lane_and_run_use_posted_native_steps_and_validate_start() {
    let iso = spawn(LANE);
    iso.probe(
        r#"
        globalThis.__plot = { bank: { x: 3235, z: 3420, level: 0 }, x0: 3235, x1: 3237, z0: 3418, z1: 3419 };
        globalThis.__here = { x: 3235, z: 3418, level: 0 };
        globalThis.__occupied = ['3236,3418'];
        "#,
    )
    .unwrap();
    let words = walkable_words(
        4,
        3,
        3235,
        3418,
        &[
            (3235, 3418),
            (3236, 3418),
            (3237, 3418),
            (3235, 3419),
            (3236, 3419),
            (3237, 3419),
        ],
    );
    let scene = api::snapshot::SceneView {
        available: true,
        base_x: 3235,
        base_z: 3418,
        level: 0,
        width: 4,
        height: 3,
        collision_flags: vec![0; 4 * 3],
    };
    let steps = api::query::pack_step_masks(&scene);
    let ranks = vec![u16::MAX; 4 * 3];
    let fires = [loc("Fire", 3235, 3418)];
    let mut snap = base();
    snap.locs = &fires;
    snap.reach = ReachViewInput {
        available: true,
        base_x: 3235,
        base_z: 3418,
        level: 0,
        width: 4,
        height: 3,
        walkable: &words,
        reachable: &words,
        reachable_adj: &words,
        exact_rank: &ranks,
        adjacent_rank: &ranks,
        step: &steps,
        canlight: &words,
        stamp: 0,
    };
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), true);
    let found = iso.probe("__found").unwrap();
    assert_eq!(found["start"]["x"], 3237);
    assert_eq!(found["start"]["z"], 3419);
    assert_eq!(found["run"], 3);
    assert_eq!(iso.probe("__in").unwrap(), true);
    assert_eq!(iso.probe("__want").unwrap(), 20);
    assert_eq!(iso.probe("__ticks").unwrap(), 1);
    assert_eq!(iso.probe("__west").unwrap(), true);
    assert_eq!(iso.probe("__refused").unwrap(), true);
    assert_eq!(
        iso.probe("__merged").unwrap(),
        serde_json::json!(["2,3", "1,2"])
    );
    iso.join();
}

#[test]
fn find_burn_lane_without_walkable_is_explicit() {
    let iso = spawn(LANE);
    iso.probe(
        r#"
        globalThis.__plot = { bank: { x: 3235, z: 3420, level: 0 }, x0: 3235, x1: 3237, z0: 3418, z1: 3419 };
        globalThis.__here = { x: 3235, z: 3418, level: 0 };
        "#,
    )
    .unwrap();
    post(&iso, &base());
    tick(&iso, 1);
    let probe = iso.probe("__ok").unwrap();
    assert!(
        probe.as_str().unwrap_or("").contains("not impl"),
        "missing walkable stays explicit, got {probe:?}"
    );
    iso.join();
}

#[test]
fn find_burn_lane_without_canlight_is_explicit() {
    let iso = spawn(LANE);
    iso.probe(
        r#"
        globalThis.__plot = { bank: { x: 3235, z: 3420, level: 0 }, x0: 3235, x1: 3237, z0: 3418, z1: 3419 };
        globalThis.__here = { x: 3235, z: 3418, level: 0 };
        "#,
    )
    .unwrap();
    let words = walkable_words(
        4,
        3,
        3235,
        3418,
        &[(3235, 3418), (3236, 3418), (3237, 3418)],
    );
    let scene = api::snapshot::SceneView {
        available: true,
        base_x: 3235,
        base_z: 3418,
        level: 0,
        width: 4,
        height: 3,
        collision_flags: vec![0; 4 * 3],
    };
    let steps = api::query::pack_step_masks(&scene);
    let ranks = vec![u16::MAX; 4 * 3];
    let mut snap = base();
    snap.reach = ReachViewInput {
        available: true,
        base_x: 3235,
        base_z: 3418,
        level: 0,
        width: 4,
        height: 3,
        walkable: &words,
        reachable: &words,
        reachable_adj: &words,
        exact_rank: &ranks,
        adjacent_rank: &ranks,
        step: &steps,
        canlight: &[],
        stamp: 0,
    };
    post(&iso, &snap);
    tick(&iso, 1);
    let probe = iso.probe("__ok").unwrap();
    assert!(
        probe.as_str().unwrap_or("").contains("not impl"),
        "missing canlight stays explicit, got {probe:?}"
    );
    iso.join();
}

#[test]
fn find_burn_lane_denies_walkable_bank_floor_from_cropped_mask() {
    let iso = spawn(LANE);
    iso.probe(
        r#"
        globalThis.__plot = { bank: { x: 3253, z: 3420, level: 0 }, x0: 3250, x1: 3261, z0: 3418, z1: 3429 };
        globalThis.__here = { x: 3252, z: 3420, level: 0 };
        "#,
    )
    .unwrap();
    let origin_x = 3240;
    let origin_z = 3410;
    let world_w = 32;
    let world_h = 32;
    let cells: usize = 4 * world_w * world_h;
    let mut bits = vec![0u64; cells.div_ceil(64)];
    let lit_x = 3261;
    let lit_z = 3429;
    let idx = (lit_z - origin_z) as usize * world_w + (lit_x - origin_x) as usize;
    bits[idx / 64] |= 1u64 << (idx % 64);
    let plane = api::query::CanlightPlane {
        bits: &bits,
        origin_x,
        origin_z,
        width: world_w as i32,
        height: world_h as i32,
    };
    let canlight = api::query::pack_canlight_u32(3250, 3418, 0, 16, 16, Some(plane));
    let walkable = walkable_words(16, 16, 3250, 3418, &[(3252, 3420), (3261, 3429)]);
    let scene = api::snapshot::SceneView {
        available: true,
        base_x: 3250,
        base_z: 3418,
        level: 0,
        width: 16,
        height: 16,
        collision_flags: vec![0; 16 * 16],
    };
    let steps = api::query::pack_step_masks(&scene);
    let ranks = vec![u16::MAX; 16 * 16];
    let mut snap = base();
    snap.reach = ReachViewInput {
        available: true,
        base_x: 3250,
        base_z: 3418,
        level: 0,
        width: 16,
        height: 16,
        walkable: &walkable,
        reachable: &walkable,
        reachable_adj: &walkable,
        exact_rank: &ranks,
        adjacent_rank: &ranks,
        step: &steps,
        canlight: &canlight,
        stamp: 0,
    };
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), true);
    let found = iso.probe("__found").unwrap();
    assert_eq!(found["start"]["x"], 3261);
    assert_eq!(found["start"]["z"], 3429);
    iso.join();
}

#[test]
fn tool_restock_plan_is_ordinary_tinderbox_withdraw() {
    let iso = spawn(RESTOCK);
    iso.probe("globalThis.__inv = {}; globalThis.__bank = { Tinderbox: 2 };")
        .unwrap();
    post(&iso, &base());
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), true);
    assert_eq!(
        iso.probe("__plan").unwrap(),
        serde_json::json!([{ "name": "Tinderbox", "qty": 1, "equip": false }])
    );
    assert_eq!(
        iso.probe("__hammer").unwrap(),
        serde_json::json!([{ "name": "Hammer", "qty": 1, "equip": false }]),
        "frozen toolRestockPlan plans any exact tool"
    );
    assert_eq!(iso.probe("__pick").unwrap(), Value::Null);
    iso.join();
}


// `localFirePlot`, `runInDir` and `NoLightTiles` are one native call each
// (`__rs2b0t_firemaking`). Expected values are the frozen
// `bot/api/firemaking/Firemaking.ts` bodies.
const HELPERS: &str = r#"
import { localFirePlot, runInDir, NoLightTiles } from '../../api/firemaking/Firemaking.js';
import Tile from '../../geometry/Tile.js';

const capture = (fn) => { try { return fn(); } catch (e) { return 'THREW ' + (e && e.name) + ': ' + (e && e.message); } };
const box = (p) => ({ bank: [p.bank.x, p.bank.z, p.bank.level], tile: p.bank instanceof Tile, x0: p.x0, x1: p.x1, z0: p.z0, z1: p.z1 });

// Frozen: `h = Math.max(2, Math.floor(half))`, `half = 8` when omitted, a box
// around the origin whatever posted plot contains it.
globalThis.__plots = {
    defaultHalf: box(localFirePlot({ x: 3253, z: 3429, level: 0 })),
    half3: box(localFirePlot({ x: 3000, z: 3000, level: 1 }, 3.7)),
    clamped: box(localFirePlot({ x: 3000, z: 3000, level: 0 }, 1)),
    nanHalf: Number.isNaN(localFirePlot({ x: 3000, z: 3000, level: 0 }, 'wide').x0),
    missing: capture(() => localFirePlot(undefined)),
};

const plot = { bank: { x: 3235, z: 3420, level: 0 }, x0: 3235, x1: 3237, z0: 3418, z1: 3419 };
const west = { dx: -1, dz: 0 };
const key = (t) => t.x + ',' + t.z;
function lane(from, occupied, walkable, canStep, cap) {
    const log = [];
    const seen = [];
    const run = capture(() => runInDir(
        from, plot, west, occupied,
        walkable && ((t) => { log.push('walkable:' + key(t)); seen.push(t); return walkable(t); }),
        canStep && ((a, b) => { log.push('canStep:' + key(a) + '>' + key(b)); return canStep(a, b); }),
        cap,
    ));
    return { run, log, firstIsFrom: seen[0] === from };
}
const start = { x: 3237, z: 3419, level: 0 };
globalThis.__lanes = {
    // Stops on the occupied third tile after two walkable/canStep rounds.
    occupied: lane(start, new Set(['3235,3419']), () => true, () => true, 5),
    // The cap ends the walk before any callback.
    capZero: lane(start, new Set(), () => true, () => true, 0),
    // A refused step counts the tile it left.
    stepRefused: lane(start, new Set(), () => true, () => false, 5),
    // Truthy non-booleans continue; the first falsy `walkable` stops.
    truthy: lane(start, new Set(), (t) => (t.x === 3235 ? 0 : 'yes'), () => 1, 5),
    // The plot edge (x0 = 3235) ends a west run of three.
    plotEdge: lane(start, new Set(), () => true, () => true, 27),
    outside: lane({ x: 9999, z: 3419, level: 0 }, new Set(), () => true, () => true, 5),
    wrongLevel: lane({ x: 3237, z: 3419, level: 1 }, new Set(), () => true, () => true, 5),
    boom: lane(start, new Set(), () => { throw new Error('walk boom'); }, () => true, 5),
    noWalkable: lane(start, new Set(), undefined, () => true, 5),
};

const a = new NoLightTiles();
const b = new NoLightTiles();
a.add({ x: 1, z: 2 });
a.add({ x: 1, z: 2 });
a.add({ x: 4, z: 5 });
const merged = a.merge(new Set(['9,9', '4,5']));
globalThis.__noLight = {
    has: a.has({ x: 1, z: 2 }),
    hasOther: a.has({ x: 2, z: 1 }),
    size: a.size,
    otherSize: b.size,
    merged: [...merged],
    mergedIsSet: merged instanceof Set,
    mergeNothing: [...a.merge(undefined)],
};
a.clear();
globalThis.__noLight.cleared = [a.size, a.has({ x: 1, z: 2 })];

export default class T extends LoopingBot { loop() {} }
"#;

#[test]
fn local_fire_plot_is_the_frozen_box_around_the_origin() {
    let iso = spawn(HELPERS);
    assert_eq!(
        iso.probe("__plots").unwrap(),
        serde_json::json!({
            "defaultHalf": {"bank": [3253, 3429, 0], "tile": true, "x0": 3245, "x1": 3261, "z0": 3421, "z1": 3437},
            "half3": {"bank": [3000, 3000, 1], "tile": true, "x0": 2997, "x1": 3003, "z0": 2997, "z1": 3003},
            "clamped": {"bank": [3000, 3000, 0], "tile": true, "x0": 2998, "x1": 3002, "z0": 2998, "z1": 3002},
            "nanHalf": true,
            "missing": "THREW TypeError: Cannot read properties of undefined (reading 'x')",
        })
    );
    iso.join();
}

#[test]
fn run_in_dir_calls_walkable_and_can_step_per_visited_tile() {
    let iso = spawn(HELPERS);
    assert_eq!(
        iso.probe("__lanes").unwrap(),
        serde_json::json!({
            "occupied": {
                "run": 2,
                "log": [
                    "walkable:3237,3419", "canStep:3237,3419>3236,3419",
                    "walkable:3236,3419", "canStep:3236,3419>3235,3419",
                ],
                "firstIsFrom": true,
            },
            "capZero": {"run": 0, "log": [], "firstIsFrom": false},
            "stepRefused": {
                "run": 1,
                "log": ["walkable:3237,3419", "canStep:3237,3419>3236,3419"],
                "firstIsFrom": true,
            },
            "truthy": {
                "run": 2,
                "log": [
                    "walkable:3237,3419", "canStep:3237,3419>3236,3419",
                    "walkable:3236,3419", "canStep:3236,3419>3235,3419",
                    "walkable:3235,3419",
                ],
                "firstIsFrom": true,
            },
            "plotEdge": {
                "run": 3,
                "log": [
                    "walkable:3237,3419", "canStep:3237,3419>3236,3419",
                    "walkable:3236,3419", "canStep:3236,3419>3235,3419",
                    "walkable:3235,3419", "canStep:3235,3419>3234,3419",
                ],
                "firstIsFrom": true,
            },
            "outside": {"run": 0, "log": [], "firstIsFrom": false},
            "wrongLevel": {"run": 0, "log": [], "firstIsFrom": false},
            "boom": {"run": "THREW Error: walk boom", "log": ["walkable:3237,3419"], "firstIsFrom": true},
            "noWalkable": {"run": "THREW TypeError: walkable is not a function", "log": [], "firstIsFrom": false},
        })
    );
    iso.join();
}

#[test]
fn no_light_tiles_keep_a_per_instance_set_in_rust() {
    let iso = spawn(HELPERS);
    assert_eq!(
        iso.probe("__noLight").unwrap(),
        serde_json::json!({
            "has": true,
            "hasOther": false,
            "size": 2,
            "otherSize": 0,
            "merged": ["9,9", "4,5", "1,2"],
            "mergedIsSet": true,
            "mergeNothing": ["1,2", "4,5"],
            "cleared": [0, false],
        })
    );
    iso.join();
}
