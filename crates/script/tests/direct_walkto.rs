// Frozen DirectNavigator (`DirectNavigator.ts:9-54`): `walk` clamps to 48
// tiles, checks the scene window and clicks on the player's plane; `walkTo`
// re-checks every two ticks and reissues the click after 2400 ms or a stall.
// A queued WalkTo is not arrival. Traveller (`walk` / `walk-near`) is a
// different host op.

use script::isolate_fb::{ReachViewInput, SnapshotInput, TileInput};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};

mod common;
use common::post_snapshot_input;

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
        reach: script::isolate_fb::ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
        self_target_kind: 0,
        self_target_index: -1,
        widgets: &[],
    }
}

fn tile(x: i32, z: i32, level: i32) -> TileInput {
    TileInput { x, z, level }
}

fn spawn(src: &str) -> LoadIsolate {
    LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn walk_to_src(call: &str) -> String {
    format!(
        r#"
import {{ DirectNavigator }} from '../../event/webwalk/DirectNavigator.js';
export default class T extends LoopingBot {{
    async loop() {{
        globalThis.__ok = null;
        try {{
            globalThis.__ok = await {call};
        }} catch (e) {{
            globalThis.__ok = String(e.message || e);
        }}
    }}
}}
"#
    )
}

/// An open 104x104 scene centred on `here`, flooded from it (the reach view
/// the host posts with every scene).
fn open_view(here: TileInput) -> api::query::ReachQueryView {
    let scene = api::snapshot::SceneView {
        available: true,
        base_x: here.x - 52,
        base_z: here.z - 52,
        level: here.level,
        width: 104,
        height: 104,
        collision_flags: vec![0; 104 * 104],
    };
    let me = api::snapshot::WorldTile {
        x: here.x,
        z: here.z,
        level: here.level,
    };
    let flood = api::query::SceneQuery::new(&scene, Some(me))
        .flood_reach()
        .expect("origin in scene");
    api::query::pack_reach_query(&scene, Some(&flood))
}

fn reach_input(view: &api::query::ReachQueryView, stamp: u64) -> ReachViewInput<'_> {
    ReachViewInput {
        available: view.available,
        base_x: view.base_x,
        base_z: view.base_z,
        level: view.level,
        width: view.width,
        height: view.height,
        walkable: &view.walkable,
        reachable: &view.reachable,
        reachable_adj: &view.reachable_adj,
        exact_rank: &view.exact_rank,
        adjacent_rank: &view.adjacent_rank,
        step: &view.step,
        canlight: &view.canlight,
        stamp,
    }
}

fn scene_walk_to(x: i32, z: i32, level: i32) -> InteractReq {
    InteractReq::WalkTo { x, z, level }
}

fn walk_src(call: &str) -> String {
    format!(
        r#"
import {{ DirectNavigator }} from '../../event/webwalk/DirectNavigator.js';
export default class T extends LoopingBot {{
    loop() {{
        if (globalThis.__done) return;
        globalThis.__done = true;
        globalThis.__ok = {call};
    }}
}}
"#
    )
}

#[test]
fn walk_to_queues_scene_walk_and_stays_pending_until_posted_arrival() {
    let iso = spawn(&walk_to_src(
        "DirectNavigator.walkTo({ x: 3222, z: 3228, level: 0 }, 0, 10000)",
    ));
    let here = tile(3222, 3222, 0);
    let view = open_view(here);
    let mut snap = base_snapshot(here);
    snap.reach = reach_input(&view, 1);
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        serde_json::Value::Null,
        "queued send is not arrival"
    );
    assert_eq!(
        iso.drain_interacts(),
        vec![scene_walk_to(3222, 3228, 0)],
        "walkTo queues scene WalkTo, not Traveller"
    );

    snap.here = Some(tile(3222, 3228, 0));
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(2);
    snap.tick = 3;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(3);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        true,
        "true once the two-tick check sees posted arrival"
    );
    assert!(
        iso.drain_interacts().is_empty(),
        "arrival does not re-queue or arm Traveller"
    );
    iso.join();
}

#[test]
fn walk_to_preserves_explicit_radius_zero_versus_default_two() {
    let here = tile(3222, 3222, 0);
    let view = open_view(here);
    let mut snap = base_snapshot(here);
    snap.reach = reach_input(&view, 1);

    let adjacent = spawn(&walk_to_src(
        "DirectNavigator.walkTo({ x: 3222, z: 3223, level: 0 })",
    ));
    post_snapshot_input(&adjacent, &snap);
    adjacent.on_game_tick(1);
    assert_eq!(
        adjacent.probe("__ok").unwrap(),
        true,
        "missing radius follows frozen DirectNavigator default 2"
    );
    assert!(
        adjacent.drain_interacts().is_empty(),
        "already within default radius 2 does not queue"
    );
    adjacent.join();

    let exact = spawn(&walk_to_src(
        "DirectNavigator.walkTo({ x: 3222, z: 3223, level: 0 }, 0, 10000)",
    ));
    post_snapshot_input(&exact, &snap);
    exact.on_game_tick(1);
    assert_eq!(
        exact.probe("__ok").unwrap(),
        serde_json::Value::Null,
        "explicit radius 0 does not treat an adjacent tile as arrival"
    );
    assert_eq!(exact.drain_interacts(), vec![scene_walk_to(3222, 3223, 0)]);
    exact.join();
}

#[test]
fn walk_to_on_another_plane_clicks_the_players_plane_and_waits_for_the_level() {
    let iso = spawn(&walk_to_src(
        "DirectNavigator.walkTo({ x: 3222, z: 3222, level: 1 })",
    ));
    let here = tile(3222, 3222, 0);
    let view = open_view(here);
    let mut snap = base_snapshot(here);
    snap.reach = reach_input(&view, 1);
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__ok").unwrap(), serde_json::Value::Null);
    assert_eq!(
        iso.drain_interacts(),
        vec![scene_walk_to(3222, 3222, 0)],
        "Input.walk(lx, lz) clicks on the player's own plane"
    );

    snap.here = Some(tile(3222, 3222, 1));
    for tick in 2..=3 {
        snap.tick = tick;
        post_snapshot_input(&iso, &snap);
        iso.on_game_tick(tick);
    }
    assert_eq!(iso.probe("__ok").unwrap(), true);
    iso.join();
}

#[test]
fn walk_to_expired_unreachable_returns_false() {
    let iso = spawn(&walk_to_src(
        "DirectNavigator.walkTo({ x: 3222, z: 3295, level: 0 }, 0, 80)",
    ));
    let here = tile(3222, 3222, 0);
    let view = open_view(here);
    let mut snap = base_snapshot(here);
    snap.reach = reach_input(&view, 1);
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__ok").unwrap(), serde_json::Value::Null);
    assert_eq!(
        iso.drain_interacts(),
        vec![scene_walk_to(3222, 3270, 0)],
        "the click is clamped 48 tiles toward the dest"
    );
    std::thread::sleep(std::time::Duration::from_millis(120));
    for tick in 2..=3 {
        snap.tick = tick;
        post_snapshot_input(&iso, &snap);
        iso.on_game_tick(tick);
    }
    assert_eq!(
        iso.probe("__ok").unwrap(),
        false,
        "unreachable dest expires false on the caller timeout, not a 45s default wait"
    );
    iso.join();
}

#[test]
fn walk_to_reissues_the_click_when_the_player_stalls() {
    let iso = spawn(&walk_to_src(
        "DirectNavigator.walkTo({ x: 3222, z: 3240, level: 0 }, 0, 10000)",
    ));
    let here = tile(3222, 3222, 0);
    let view = open_view(here);
    let mut snap = base_snapshot(here);
    snap.reach = reach_input(&view, 1);
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__ok").unwrap(), serde_json::Value::Null);
    assert_eq!(iso.drain_interacts(), vec![scene_walk_to(3222, 3240, 0)]);
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(2);
    assert_eq!(iso.probe("__ok").unwrap(), serde_json::Value::Null);
    assert!(iso.drain_interacts().is_empty(), "checks every two ticks");
    snap.tick = 3;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(3);
    assert_eq!(iso.probe("__ok").unwrap(), serde_json::Value::Null);
    assert_eq!(
        iso.drain_interacts(),
        vec![scene_walk_to(3222, 3240, 0)],
        "a player that did not move gets the click again"
    );
    iso.join();
}

#[test]
fn walk_clamps_to_48_tiles_and_refuses_outside_the_scene() {
    let here = tile(3222, 3222, 0);
    let view = open_view(here);
    let mut snap = base_snapshot(here);
    snap.reach = reach_input(&view, 1);

    let far = spawn(&walk_src(
        "DirectNavigator.walk({ x: 3322, z: 3200, level: 0 })",
    ));
    post_snapshot_input(&far, &snap);
    far.on_game_tick(1);
    assert_eq!(far.probe("__ok").unwrap(), true);
    assert_eq!(
        far.drain_interacts(),
        vec![scene_walk_to(3270, 3200, 0)],
        "the click is clamped to 48 tiles on each axis"
    );
    far.join();

    let edge = api::query::ReachQueryView {
        base_x: 3222 - 100,
        ..view.clone()
    };
    snap.reach = reach_input(&edge, 2);
    let outside = spawn(&walk_src(
        "DirectNavigator.walk({ x: 3322, z: 3222, level: 0 })",
    ));
    post_snapshot_input(&outside, &snap);
    outside.on_game_tick(1);
    assert_eq!(
        outside.probe("__ok").unwrap(),
        false,
        "a clamped tile outside the scene window has no local tile"
    );
    assert!(outside.drain_interacts().is_empty());
    outside.join();
}

#[test]
fn walk_to_stop_does_not_count_queued_send_as_arrival() {
    // Pre-existing Stop seam: ThreadMsg::Stopped clears the interact
    // queue, so a queued WalkTo is not observable after stop. The wait
    // must not resolve true. Do not add a new abort path.
    let src = r#"
import { DirectNavigator } from '../../event/webwalk/DirectNavigator.js';
import { ScriptRunner } from '../../runtime/ScriptRunner.js';
export default class T extends LoopingBot {
    async loop() {
        globalThis.__ok = null;
        const pending = DirectNavigator.walkTo({ x: 3222, z: 3295, level: 0 }, 0, 10000);
        ScriptRunner.stop('abort walkTo');
        globalThis.__ok = await pending;
        this.log('walk-result:' + String(globalThis.__ok));
    }
}
"#;
    let iso = spawn(src);
    let snap = base_snapshot(tile(3222, 3222, 0));
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let receipt = loop {
        if let Some(receipt) = iso.script_stop_receipt() {
            break receipt;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "ScriptRunner.stop did not terminate the parked walkTo"
        );
        std::thread::yield_now();
    };
    assert_eq!(receipt.reason, "abort walkTo");
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    iso.join_detached(done_tx);
    let logs = done_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("walkTo stop join exceeded the generous hang guard");
    assert!(
        logs.iter().any(|l| l.contains("script requested stop")),
        "ScriptRunner.stop must log while walkTo is parked: {logs:?}"
    );
    assert!(
        !logs.iter().any(|l| l == "walk-result:true"),
        "Stop must end the isolate without resolving walkTo true: {logs:?}"
    );
}

#[test]
fn walk_to_invalid_dest_is_false_without_queue() {
    let iso = spawn(&walk_to_src("DirectNavigator.walkTo(null)"));
    let snap = base_snapshot(tile(3222, 3222, 0));
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__ok").unwrap(), false);
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}
