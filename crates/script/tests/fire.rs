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
            globalThis.__found = findBurnLane(plot, globalThis.__here, occupied);
            globalThis.__in = inFirePlot(globalThis.__here, plot);
            globalThis.__want = burnLaneWant(20);
            globalThis.__ticks = fireReactionTicks();
            globalThis.__west = isBurnWest({ dx: -1, dz: 0 });
            const no = new NoLightTiles();
            no.add({ x: 1, z: 2 });
            globalThis.__refused = no.has({ x: 1, z: 2 }) && no.size === 1 && tileKey({ x: 1, z: 2 }) === '1,2';
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
            try { toolRestockPlan([exactTool('Hammer')], () => 1, () => 0, () => 1); globalThis.__hammer = 'called'; }
            catch (e) { globalThis.__hammer = String(e.message || e); }
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
fn find_burn_lane_returns_one_walkable_tile_skipping_fire_and_refused() {
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
        &[(3235, 3418), (3236, 3418), (3237, 3418), (3235, 3419)],
    );
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
        step: &[],
    };
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), true);
    let found = iso.probe("__found").unwrap();
    assert_eq!(found["start"]["x"], 3235);
    assert_eq!(found["start"]["z"], 3419);
    assert_eq!(found["run"], 1);
    assert_eq!(iso.probe("__in").unwrap(), true);
    assert_eq!(iso.probe("__want").unwrap(), 20);
    assert_eq!(iso.probe("__ticks").unwrap(), 1);
    assert_eq!(iso.probe("__west").unwrap(), true);
    assert_eq!(iso.probe("__refused").unwrap(), true);
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
    assert!(
        iso.probe("__hammer")
            .unwrap()
            .as_str()
            .unwrap_or("")
            .contains("not impl"),
        "hammer/gatherer restock stays not impl"
    );
    assert_eq!(iso.probe("__pick").unwrap(), Value::Null);
    iso.join();
}
