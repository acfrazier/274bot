// ClientAdapter.toLocal / actions.walkTo are thin scene-shape mappings
// onto posted reach facts and the existing scene walk-to queue.
// Queue admission is not arrival. Traveller (`walk` / `walk-near`) is
// a different host op.

use script::isolate_fb::{ReachViewInput, SnapshotInput, TileInput};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};

fn post_snapshot_input(iso: &LoadIsolate, input: &SnapshotInput<'_>) {
    iso.post_snapshot(script::isolate_fb::encode_snapshot(input));
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
        side_tab: -1,
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

fn tile(x: i32, z: i32, level: i32) -> TileInput {
    TileInput { x, z, level }
}

fn posted_scene(
    base_x: i32,
    base_z: i32,
    level: i32,
    width: i32,
    height: i32,
) -> ReachViewInput<'static> {
    ReachViewInput {
        available: true,
        base_x,
        base_z,
        level,
        width,
        height,
        walkable: &[],
        reachable: &[],
        reachable_adj: &[],
        step: &[],
    }
}

fn spawn(src: &str) -> LoadIsolate {
    LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn probe_src(body: &str) -> String {
    format!(
        r#"
import {{ reader, actions }} from '../../adapter/ClientAdapter.js';
export default class T extends LoopingBot {{
    loop() {{
        {body}
    }}
}}
"#
    )
}

fn scene_walk_to(x: i32, z: i32, level: i32) -> InteractReq {
    InteractReq::WalkTo { x, z, level }
}

#[test]
fn to_local_maps_nonzero_base_and_104_edges() {
    let iso = spawn(&probe_src(
        r#"
        globalThis.__probe = {
            origin: reader.toLocal(3222, 3228),
            inside: reader.toLocal(3222 + 5, 3228 + 10),
            sw: reader.toLocal(3222, 3228),
            ne: reader.toLocal(3222 + 103, 3228 + 103),
            west: reader.toLocal(3221, 3228),
            east: reader.toLocal(3222 + 104, 3228),
            south: reader.toLocal(3222, 3227),
            north: reader.toLocal(3222, 3228 + 104),
        };
        "#,
    ));
    let mut snap = base_snapshot(tile(3222, 3228, 0));
    snap.reach = posted_scene(3222, 3228, 0, 104, 104);
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let value = iso.probe("__probe").unwrap();
    assert_eq!(value["origin"]["lx"], 0);
    assert_eq!(value["origin"]["lz"], 0);
    assert_eq!(value["inside"]["lx"], 5);
    assert_eq!(value["inside"]["lz"], 10);
    assert_eq!(value["sw"]["lx"], 0);
    assert_eq!(value["sw"]["lz"], 0);
    assert_eq!(value["ne"]["lx"], 103);
    assert_eq!(value["ne"]["lz"], 103);
    assert_eq!(value["west"], serde_json::Value::Null);
    assert_eq!(value["east"], serde_json::Value::Null);
    assert_eq!(value["south"], serde_json::Value::Null);
    assert_eq!(value["north"], serde_json::Value::Null);
    assert!(
        iso.drain_interacts().is_empty(),
        "toLocal is a sync read, no queued command"
    );
    iso.join();
}

#[test]
fn to_local_rejects_wrong_inputs_and_unavailable_old_data() {
    let iso = spawn(&probe_src(
        r#"
        globalThis.__probe = {
            missing: reader.toLocal(),
            nonInt: reader.toLocal(3222.5, 3228),
            nan: reader.toLocal(Number.NaN, 3228),
            inf: reader.toLocal(3222, Number.POSITIVE_INFINITY),
            str: reader.toLocal('3222', 3228),
            swappedOut: reader.toLocal(3228, 3222),
        };
        "#,
    ));
    let mut snap = base_snapshot(tile(3222, 3228, 0));
    snap.reach = posted_scene(3222, 3228, 0, 104, 104);
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let value = iso.probe("__probe").unwrap();
    assert_eq!(value["missing"], serde_json::Value::Null);
    assert_eq!(value["nonInt"], serde_json::Value::Null);
    assert_eq!(value["nan"], serde_json::Value::Null);
    assert_eq!(value["inf"], serde_json::Value::Null);
    assert_eq!(value["str"], serde_json::Value::Null);
    assert_eq!(
        value["swappedOut"],
        serde_json::Value::Null,
        "z as x is out of the posted scene"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();

    let stale = spawn(&probe_src(
        r#"
        globalThis.__probe = {
            local: reader.toLocal(5, 5),
            inBase: reader.toLocal(3222, 3228),
        };
        globalThis.__walk = actions.walkTo(5, 5);
        "#,
    ));
    let snap = base_snapshot(tile(3222, 3228, 0));
    post_snapshot_input(&stale, &snap);
    stale.on_game_tick(1);
    let value = stale.probe("__probe").unwrap();
    assert_eq!(
        value["local"],
        serde_json::Value::Null,
        "unavailable reach must not convert through a zero base"
    );
    assert_eq!(value["inBase"], serde_json::Value::Null);
    assert_eq!(stale.probe("__walk").unwrap(), false);
    assert!(
        stale.drain_interacts().is_empty(),
        "unavailable query/action must queue nothing"
    );
    stale.join();
}

#[test]
fn walk_to_queues_world_coords_on_current_plane_without_waiting() {
    let iso = spawn(&probe_src(
        r#"
        globalThis.__ok = actions.walkTo(5, 10);
        "#,
    ));
    let mut snap = base_snapshot(tile(3222, 3228, 0));
    snap.reach = posted_scene(3222, 3228, 1, 104, 104);
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        true,
        "queued send returns immediately; arrival is the caller's wait"
    );
    assert_eq!(
        iso.drain_interacts(),
        vec![scene_walk_to(3227, 3238, 1)],
        "walkTo queues scene WalkTo at world = base + local, current plane"
    );
    iso.join();
}

#[test]
fn walk_to_queues_nothing_outside_scene_or_on_bad_input() {
    let iso = spawn(&probe_src(
        r#"
        globalThis.__probe = {
            neg: actions.walkTo(-1, 0),
            past: actions.walkTo(104, 0),
            zPast: actions.walkTo(0, 104),
            nonInt: actions.walkTo(5.5, 10),
            missing: actions.walkTo(),
            origin: actions.walkTo(0, 0),
            ne: actions.walkTo(103, 103),
        };
        "#,
    ));
    let mut snap = base_snapshot(tile(3222, 3228, 0));
    snap.reach = posted_scene(3222, 3228, 0, 104, 104);
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let value = iso.probe("__probe").unwrap();
    assert_eq!(value["neg"], false);
    assert_eq!(value["past"], false);
    assert_eq!(value["zPast"], false);
    assert_eq!(value["nonInt"], false);
    assert_eq!(value["missing"], false);
    assert_eq!(value["origin"], true);
    assert_eq!(value["ne"], true);
    assert_eq!(
        iso.drain_interacts(),
        vec![scene_walk_to(3222, 3228, 0), scene_walk_to(3325, 3331, 0)],
        "only in-scene integer locals queue, in call order"
    );
    iso.join();
}

#[test]
fn flax_pair_to_local_then_walk_to_queues_exact_world() {
    let iso = spawn(&probe_src(
        r#"
        const local = reader.toLocal(3230, 3234);
        globalThis.__probe = {
            local,
            queued: local ? actions.walkTo(local.lx, local.lz) : null,
        };
        "#,
    ));
    let mut snap = base_snapshot(tile(3222, 3228, 0));
    snap.reach = posted_scene(3222, 3228, 0, 104, 104);
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let value = iso.probe("__probe").unwrap();
    assert_eq!(value["local"]["lx"], 8);
    assert_eq!(value["local"]["lz"], 6);
    assert_eq!(value["queued"], true);
    assert_eq!(
        iso.drain_interacts(),
        vec![scene_walk_to(3230, 3234, 0)],
        "toLocal then walkTo queues one scene WalkTo at the world dest"
    );
    iso.join();
}

#[test]
fn current_base_and_plane_change_affects_the_next_call() {
    let iso = spawn(&probe_src(
        r#"
        const local = reader.toLocal(3230, 3234);
        globalThis.__probe = {
            local,
            queued: local ? actions.walkTo(local.lx, local.lz) : false,
            leftover: actions.walkTo(8, 6),
        };
        "#,
    ));
    let mut snap = base_snapshot(tile(3222, 3228, 0));
    snap.reach = posted_scene(3222, 3228, 0, 104, 104);
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let first = iso.probe("__probe").unwrap();
    assert_eq!(first["local"]["lx"], 8);
    assert_eq!(first["queued"], true);
    assert_eq!(
        iso.drain_interacts(),
        vec![scene_walk_to(3230, 3234, 0), scene_walk_to(3230, 3234, 0)]
    );

    snap.tick = 2;
    snap.here = Some(tile(4000, 4100, 2));
    snap.reach = posted_scene(4000, 4100, 2, 104, 104);
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(2);
    let second = iso.probe("__probe").unwrap();
    assert_eq!(
        second["local"],
        serde_json::Value::Null,
        "old world dest is outside the new scene"
    );
    assert_eq!(second["queued"], false);
    assert_eq!(
        iso.drain_interacts(),
        vec![scene_walk_to(4008, 4106, 2)],
        "stale local 8,6 now means the new base/plane"
    );

    let moved = spawn(&probe_src(
        r#"
        const local = reader.toLocal(4008, 4106);
        globalThis.__probe = {
            local,
            queued: local ? actions.walkTo(local.lx, local.lz) : false,
        };
        "#,
    ));
    post_snapshot_input(&moved, &snap);
    moved.on_game_tick(2);
    let value = moved.probe("__probe").unwrap();
    assert_eq!(value["local"]["lx"], 8);
    assert_eq!(value["local"]["lz"], 6);
    assert_eq!(value["queued"], true);
    assert_eq!(moved.drain_interacts(), vec![scene_walk_to(4008, 4106, 2)]);
    iso.join();
    moved.join();
}

#[test]
fn two_slots_do_not_exchange_scene_bases() {
    let src = probe_src(
        r#"
        const local = reader.toLocal(3230, 3234);
        globalThis.__probe = {
            local,
            queued: actions.walkTo(8, 6),
        };
        "#,
    );
    let a = spawn(&src);
    let b = spawn(&src);
    let mut snap_a = base_snapshot(tile(3222, 3228, 0));
    snap_a.reach = posted_scene(3222, 3228, 0, 104, 104);
    let mut snap_b = base_snapshot(tile(4000, 4100, 1));
    snap_b.reach = posted_scene(4000, 4100, 1, 104, 104);
    post_snapshot_input(&a, &snap_a);
    post_snapshot_input(&b, &snap_b);
    a.on_game_tick(1);
    b.on_game_tick(1);
    let pa = a.probe("__probe").unwrap();
    let pb = b.probe("__probe").unwrap();
    assert_eq!(pa["local"]["lx"], 8);
    assert_eq!(pa["local"]["lz"], 6);
    assert_eq!(pb["local"], serde_json::Value::Null);
    assert_eq!(pa["queued"], true);
    assert_eq!(pb["queued"], true);
    assert_eq!(a.drain_interacts(), vec![scene_walk_to(3230, 3234, 0)]);
    assert_eq!(
        b.drain_interacts(),
        vec![scene_walk_to(4008, 4106, 1)],
        "slot B must not inherit slot A's base"
    );
    a.join();
    b.join();
}

#[test]
fn omitted_delta_keeps_last_scene_unavailable_clears() {
    let iso = spawn(&probe_src(
        r#"
        const local = reader.toLocal(3230, 3234);
        globalThis.__probe = {
            local,
            queued: local ? actions.walkTo(local.lx, local.lz) : false,
            zeroBase: reader.toLocal(5, 5),
        };
        "#,
    ));
    let mut snap = base_snapshot(tile(3222, 3228, 0));
    snap.reach = posted_scene(3222, 3228, 0, 104, 104);
    let (keyframe, fp) = script::isolate_fb::encode_snapshot_delta(None, &snap, false);
    iso.post_snapshot(keyframe);
    iso.on_game_tick(1);
    let first = iso.probe("__probe").unwrap();
    assert_eq!(first["local"]["lx"], 8);
    assert_eq!(first["queued"], true);
    assert_eq!(iso.drain_interacts(), vec![scene_walk_to(3230, 3234, 0)]);

    snap.tick = 2;
    let (delta, fp2) = script::isolate_fb::encode_snapshot_delta(Some(&fp), &snap, false);
    let omitted = script::isolate_fb::decode_snapshot(&delta).expect("delta");
    assert!(!omitted.has_reach(), "unchanged reach omitted");
    iso.post_snapshot(delta);
    iso.on_game_tick(2);
    let kept = iso.probe("__probe").unwrap();
    assert_eq!(
        kept["local"]["lx"], 8,
        "omitted delta keeps last scene base"
    );
    assert_eq!(kept["queued"], true);
    assert_eq!(iso.drain_interacts(), vec![scene_walk_to(3230, 3234, 0)]);

    snap.tick = 3;
    snap.reach = ReachViewInput::UNAVAILABLE;
    let (cleared, _) = script::isolate_fb::encode_snapshot_delta(Some(&fp2), &snap, false);
    iso.post_snapshot(cleared);
    iso.on_game_tick(3);
    let gone = iso.probe("__probe").unwrap();
    assert_eq!(gone["local"], serde_json::Value::Null);
    assert_eq!(gone["queued"], false);
    assert_eq!(
        gone["zeroBase"],
        serde_json::Value::Null,
        "unavailable must not convert (5,5) through a zero base"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}
