//! Named `Bank.openNearest` walks the selected loc with existing WalkNear
//! radius 1, then posts the same named OpenBooth identity.

use script::isolate_fb::{
    NearestBoothInput, ReachViewInput, SceneEntityInput, SnapshotInput, TileInput,
};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};

fn post_snapshot_input(iso: &LoadIsolate, input: &SnapshotInput<'_>) {
    iso.post_snapshot(script::isolate_fb::encode_snapshot(input));
}

fn base_snapshot<'a>() -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: Some(TileInput {
            x: 3010,
            z: 3352,
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

fn tile(x: i32, z: i32) -> TileInput {
    TileInput { x, z, level: 0 }
}

fn tick(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    let _ = iso.probe("true");
}

const OPEN_NEAREST: &str = r#"
import { Bank } from '../../api/bank/Bank.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        globalThis.__ok = await Bank.openNearest('Bank booth', 'Use-quickly');
    }
}
"#;

const BANKING_OPEN: &str = r#"
import { Banking } from '../../api/bank/Banking.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = await Banking.open();
    }
}
"#;

fn east_booth<'a>(actions: &'a [String]) -> SceneEntityInput<'a> {
    loc_row(2213, Some("Bank booth"), 3011, 3354, 2, actions)
}

fn named_open_booth() -> InteractReq {
    InteractReq::OpenBooth {
        x: 3011,
        z: 3354,
        level: 0,
        id: 2213,
        name: Some("Bank booth".into()),
        action: Some("Use-quickly".into()),
    }
}

fn walk_near_selected() -> InteractReq {
    InteractReq::WalkNear {
        x: 3011,
        z: 3354,
        level: 0,
        radius: 1,
        allow_teleports: false,
    }
}

#[test]
fn named_open_nearest_from_chebyshev_2_queues_walk_near_then_same_named_open_booth() {
    let iso = LoadIsolate::spawn(OPEN_NEAREST.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let use_quickly = ["Use-quickly".to_string()];
    let locs = [east_booth(&use_quickly)];
    let mut snap = base_snapshot();
    snap.locs = &locs;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), serde_json::Value::Null);
    assert_eq!(
        iso.drain_interacts(),
        vec![walk_near_selected()],
        "Chebyshev 2 must WalkNear the selected loc, not a lone OpenBooth"
    );

    snap.tick = 2;
    snap.here = Some(tile(3011, 3353));
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(
        iso.drain_interacts(),
        vec![named_open_booth()],
        "click must reuse the same id/name/action/tile, not a later nearest"
    );
    iso.join();
}

#[test]
fn named_open_nearest_from_chebyshev_1_queues_named_open_booth_only() {
    let iso = LoadIsolate::spawn(OPEN_NEAREST.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let use_quickly = ["Use-quickly".to_string()];
    let locs = [loc_row(
        2213,
        Some("Bank booth"),
        3011,
        3354,
        1,
        &use_quickly,
    )];
    let mut snap = base_snapshot();
    snap.here = Some(tile(3011, 3353));
    snap.locs = &locs;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![named_open_booth()],
        "already-adjacent named open must not WalkNear"
    );
    iso.join();
}

#[test]
fn named_open_nearest_missing_loc_after_approach_sends_no_click() {
    let iso = LoadIsolate::spawn(OPEN_NEAREST.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let use_quickly = ["Use-quickly".to_string()];
    let locs = [east_booth(&use_quickly)];
    let mut snap = base_snapshot();
    snap.locs = &locs;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![walk_near_selected()]);

    let empty: [SceneEntityInput; 0] = [];
    snap.tick = 2;
    snap.here = Some(tile(3011, 3353));
    snap.locs = &empty;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert!(
        iso.drain_interacts().is_empty(),
        "missing loc after approach must not invent a click"
    );
    assert_eq!(iso.probe("__ok").unwrap(), false);
    iso.join();
}

#[test]
fn named_open_nearest_replaced_loc_after_approach_sends_no_click() {
    let iso = LoadIsolate::spawn(OPEN_NEAREST.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let use_quickly = ["Use-quickly".to_string()];
    let locs = [east_booth(&use_quickly)];
    let mut snap = base_snapshot();
    snap.locs = &locs;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![walk_near_selected()]);

    let replaced = [
        loc_row(9999, Some("Bank booth"), 3011, 3354, 1, &use_quickly),
        loc_row(2213, Some("Bank booth"), 3012, 3353, 1, &use_quickly),
    ];
    snap.tick = 2;
    snap.here = Some(tile(3011, 3353));
    snap.locs = &replaced;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert!(
        iso.drain_interacts().is_empty(),
        "replaced id or a later matching booth must not receive the click"
    );
    assert_eq!(iso.probe("__ok").unwrap(), false);
    iso.join();
}

#[test]
fn unnamed_banking_open_from_distance_queues_open_booth_without_walk() {
    let iso = LoadIsolate::spawn(BANKING_OPEN.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = base_snapshot();
    snap.here = Some(tile(100, 100));
    snap.booths = &[TileInput {
        x: 200,
        z: 100,
        level: 0,
    }];
    snap.nearest_booth = Some(NearestBoothInput {
        x: 200,
        z: 100,
        level: 0,
        id: 2213,
        name: "Bank booth",
        op: "Use-quickly",
    });
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::OpenBooth {
            x: 200,
            z: 100,
            level: 0,
            id: 2213,
            name: None,
            action: None,
        }],
        "unnamed Banking.open stays click-only"
    );
    iso.join();
}

#[test]
fn named_open_nearest_completes_on_fresh_generation_not_queued_click() {
    let iso = LoadIsolate::spawn(OPEN_NEAREST.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let use_quickly = ["Use-quickly".to_string()];
    let locs = [loc_row(
        2213,
        Some("Bank booth"),
        3011,
        3354,
        1,
        &use_quickly,
    )];
    let mut snap = base_snapshot();
    snap.here = Some(tile(3011, 3353));
    snap.locs = &locs;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), serde_json::Value::Null);
    assert_eq!(iso.drain_interacts(), vec![named_open_booth()]);

    snap.tick = 2;
    snap.bank_open = true;
    snap.bank_loaded = true;
    snap.bank_generation = 1;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.probe("__ok").unwrap(), true);
    assert!(
        iso.drain_interacts().is_empty(),
        "fresh generation completes the wait; it does not re-click"
    );
    iso.join();
}

#[test]
fn open_nearest_access_non_default_still_throws_unsupported() {
    let src = r#"
import { Bank } from '../../api/bank/Bank.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        try {
            await Bank.openNearestAccess({ name: 'Bank chest', op: 'Use' });
            globalThis.__err = null;
        } catch (e) {
            globalThis.__err = String(e.message || e);
        }
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let snap = base_snapshot();
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    let err = iso.probe("__err").unwrap();
    assert_eq!(
        err,
        "not impl: Bank.openNearestAccess: unsupported bank access"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}
