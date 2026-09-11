// DirectNavigator.walkTo is a thin positional mapping onto the existing
// Traversal scene-walk/wait. A queued WalkTo is not arrival. Traveller
// (`walk` / `walk-near`) is a different host op.

use script::isolate_fb::{SnapshotInput, TileInput};
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
        reach: script::isolate_fb::ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
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

fn scene_walk_to(x: i32, z: i32, level: i32) -> InteractReq {
    InteractReq::WalkTo { x, z, level }
}

#[test]
fn walk_to_queues_scene_walk_and_stays_pending_until_posted_arrival() {
    let iso = spawn(&walk_to_src(
        "DirectNavigator.walkTo({ x: 3222, z: 3228, level: 0 }, 0, 10000)",
    ));
    let mut snap = base_snapshot(tile(3222, 3222, 0));
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
    assert_eq!(
        iso.probe("__ok").unwrap(),
        true,
        "true only after posted coordinates satisfy the Traversal chebyshev wait"
    );
    assert!(
        iso.drain_interacts().is_empty(),
        "arrival does not re-queue or arm Traveller"
    );
    iso.join();
}

#[test]
fn walk_to_preserves_explicit_radius_zero_versus_default_two() {
    let adjacent = spawn(&walk_to_src(
        "DirectNavigator.walkTo({ x: 3222, z: 3223, level: 0 })",
    ));
    let snap = base_snapshot(tile(3222, 3222, 0));
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
fn walk_to_does_not_succeed_on_a_different_plane() {
    let iso = spawn(&walk_to_src(
        "DirectNavigator.walkTo({ x: 3222, z: 3222, level: 1 })",
    ));
    let mut snap = base_snapshot(tile(3222, 3222, 0));
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__ok").unwrap(), serde_json::Value::Null);
    assert_eq!(
        iso.drain_interacts(),
        vec![scene_walk_to(3222, 3222, 1)],
        "other-plane dest still queues scene WalkTo, never Traveller"
    );

    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(2);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        serde_json::Value::Null,
        "same x/z on another plane is not arrival"
    );

    snap.here = Some(tile(3222, 3222, 1));
    snap.tick = 3;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(3);
    assert_eq!(iso.probe("__ok").unwrap(), true);
    iso.join();
}

#[test]
fn walk_to_expired_unreachable_returns_false() {
    let iso = spawn(&walk_to_src(
        "DirectNavigator.walkTo({ x: 3222, z: 3295, level: 0 }, 0, 80)",
    ));
    let snap = base_snapshot(tile(3222, 3222, 0));
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__ok").unwrap(), serde_json::Value::Null);
    assert_eq!(iso.drain_interacts(), vec![scene_walk_to(3222, 3295, 0)]);
    std::thread::sleep(std::time::Duration::from_millis(120));
    iso.on_game_tick(2);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        false,
        "unreachable dest expires false on the caller timeout, not a 45s default wait"
    );
    iso.join();
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
    }
}
"#;
    let iso = spawn(src);
    let snap = base_snapshot(tile(3222, 3222, 0));
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let logs = loop {
        let logs = iso.drain_logs();
        if logs.iter().any(|l| l.contains("stop")) || std::time::Instant::now() > deadline {
            break logs;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert!(
        logs.iter().any(|l| l.contains("stop")),
        "ScriptRunner.stop must log while walkTo is parked: {logs:?}"
    );
    loop {
        if iso.probe("1 + 1").is_err() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Stop must end the isolate without resolving walkTo true"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(iso.stopped());
    iso.join();
}

#[test]
fn walk_to_session_reset_drops_unforwarded_sends_but_parked_wait_survives() {
    // Pre-existing Execution wait limitation: reset_session_work clears the
    // interact queue and does not reject a parked delayUntil. This mapping
    // does not invent a new abort.
    let iso = spawn(&walk_to_src(
        "DirectNavigator.walkTo({ x: 3222, z: 3295, level: 0 }, 0, 10000)",
    ));
    let mut snap = base_snapshot(tile(3222, 3222, 0));
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        serde_json::Value::Null,
        "queued send is not arrival"
    );
    assert_eq!(iso.drain_interacts(), vec![scene_walk_to(3222, 3295, 0)]);
    iso.reset_session_work();
    assert_eq!(
        iso.probe("__ok").unwrap(),
        serde_json::Value::Null,
        "session abort does not resolve the parked wait"
    );
    assert!(iso.drain_interacts().is_empty());

    snap.here = Some(tile(3222, 3295, 0));
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(2);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        true,
        "parked Traversal wait still observes later posted arrival"
    );
    iso.join();
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
