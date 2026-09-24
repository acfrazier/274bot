//! Reach.npcDialog: shim marshals into the Rust sequencer; queued verbs are
//! the existing FlatBuffer walk-near / npc actions.

use script::isolate_fb::{
    encode_snapshot, encode_snapshot_with_native, ChatLineInput, NativeFactsInput, ReachViewInput,
    SceneEntityInput, SnapshotInput, TileInput,
};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};
use serde_json::Value;

fn spawn(src: &str) -> LoadIsolate {
    LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn npc<'a>(
    name: &'a str,
    actions: &'a [String],
    index: i32,
    x: i32,
    z: i32,
    distance: i32,
    reachable_adj: bool,
) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index,
        id: 1,
        name: Some(name),
        x,
        z,
        level: 0,
        distance,
        health: 100,
        max_health: 100,
        in_combat: false,
        animating: false,
        actions,
        reachable: reachable_adj,
        reachable_adj,
        combat_level: 0,
        target_kind: 1,
        target_index: -1,
        size: 0,
        nx: 0,
        nz: 0,
    }
}

fn base<'a>(here: TileInput) -> SnapshotInput<'a> {
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

fn stand() -> TileInput {
    TileInput {
        x: 5,
        z: 5,
        level: 0,
    }
}

fn post(iso: &LoadIsolate, snap: &SnapshotInput<'_>) {
    iso.post_snapshot(encode_snapshot(snap));
}

fn post_native(iso: &LoadIsolate, snap: &SnapshotInput<'_>, native: NativeFactsInput<'_>) {
    iso.post_snapshot(encode_snapshot_with_native(snap, native));
}

fn tick(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    let _ = iso.probe("true");
}

fn fail_native(request_id: u64, x: i32, z: i32, radius: i32) -> NativeFactsInput<'static> {
    NativeFactsInput {
        walk_outcome_seq: 1,
        walk_outcome_generation: 1,
        walk_outcome_request_id: request_id,
        walk_outcome_failed: true,
        walk_outcome_x: x,
        walk_outcome_z: z,
        walk_outcome_level: 0,
        walk_outcome_radius: radius,
        walk_outcome_allow_teleports: false,
        ..Default::default()
    }
}

const NPC_DIALOG: &str = r#"
import { Reach } from '../../api/walking/Reach.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        try {
            globalThis.__ok = await Reach.npcDialog({
                name: globalThis.__npc || 'Traiborn',
                near: globalThis.__near || { x: 5, z: 5, level: 0 },
                openMs: globalThis.__openMs,
                log: (m) => { globalThis.__logs = (globalThis.__logs || []).concat(String(m)); },
            });
        } catch (e) {
            globalThis.__ok = String(e.message || e);
        }
    }
}
"#;

const SURFACE: &str = r#"
import { Reach } from '../../api/walking/Reach.js';
export default class T extends LoopingBot {
    async loop() {
        globalThis.__npcDialog = typeof Reach.npcDialog;
        globalThis.__entityOp = typeof Reach.entityOp;
        if (!globalThis.__entity) return;
        globalThis.__entityStatus = await Reach.entityOp({
            find: () => ({ interact: () => true }),
            op: 'Attack',
            expect: () => true,
        });
    }
}
"#;

#[test]
fn shim_exports_npc_dialog_and_keeps_entity_op() {
    let iso = spawn(SURFACE);
    let snap = base(stand());
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__npcDialog").unwrap(), "function");
    assert_eq!(iso.probe("__entityOp").unwrap(), "function");
    iso.probe("globalThis.__entity = true").unwrap();
    tick(&iso, 2);
    // Frozen: an already-true expect still waits for the next pump.
    tick(&iso, 3);
    assert_eq!(iso.probe("__entityStatus").unwrap(), "done");
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn open_chat_adjacent_name_without_talk_is_done_and_emits_nothing() {
    let iso = spawn(NPC_DIALOG);
    let actions = ["Bank".to_string()];
    let npcs = [npc("Traiborn", &actions, 3, 5, 6, 1, true)];
    let mut snap = base(stand());
    snap.npcs = &npcs;
    snap.chat_modal_id = 968;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), "done");
    assert!(
        iso.drain_interacts().is_empty(),
        "owned open chat must not walk or Talk-to"
    );
    iso.join();
}

#[test]
fn missing_npc_queues_close_in_walk_near_with_request_id() {
    let iso = spawn(NPC_DIALOG);
    let mut snap = base(TileInput {
        x: 0,
        z: 0,
        level: 0,
    });
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);
    let request_id = match &iso.drain_interacts()[..] {
        [InteractReq::WalkNear {
            x: 5,
            z: 5,
            level: 0,
            radius: 3,
            allow_teleports: false,
            allow_wilderness: true,
            allow_bank_fetch: true,
            request_id,
        }] => {
            assert_ne!(*request_id, 0);
            *request_id
        }
        other => panic!("expected close-in walk-near, got {other:?}"),
    };
    snap.tick = 2;
    post_native(&iso, &snap, fail_native(request_id, 5, 5, 3));
    tick(&iso, 2);
    assert_eq!(iso.probe("__ok").unwrap(), "retry");
    iso.join();
}

#[test]
fn stand_generic_fail_still_queues_npc_talk() {
    let iso = spawn(NPC_DIALOG);
    let actions = ["Talk-to".to_string()];
    let npcs = [npc("Traiborn", &actions, 9, 8, 5, 3, false)];
    let mut snap = base(TileInput {
        x: 0,
        z: 0,
        level: 0,
    });
    snap.npcs = &npcs;
    post(&iso, &snap);
    tick(&iso, 1);
    let request_id = match &iso.drain_interacts()[..] {
        [InteractReq::WalkNear {
            x: 5,
            z: 5,
            radius: 1,
            request_id,
            ..
        }] => *request_id,
        other => panic!("expected stand walk-near, got {other:?}"),
    };
    snap.tick = 2;
    post_native(&iso, &snap, fail_native(request_id, 5, 5, 1));
    tick(&iso, 2);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Npc {
            name: "Traiborn".into(),
            action: "Talk-to".into(),
            index: Some(9),
        }]
    );
    iso.join();
}

#[test]
fn talk_then_new_modal_is_done() {
    let iso = spawn(NPC_DIALOG);
    let actions = ["Examine".to_string(), "Talk-to".to_string()];
    let npcs = [npc("Traiborn", &actions, 4, 6, 5, 1, true)];
    let mut snap = base(stand());
    snap.npcs = &npcs;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Npc {
            name: "Traiborn".into(),
            action: "Talk-to".into(),
            index: Some(4),
        }]
    );
    snap.tick = 2;
    snap.chat_modal_id = 241;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.probe("__ok").unwrap(), "done");
    iso.join();
}

#[test]
fn leftover_continue_and_cant_reach_do_not_settle() {
    let iso = spawn(NPC_DIALOG);
    let actions = ["Talk-to".to_string()];
    let npcs = [npc("Traiborn", &actions, 4, 6, 5, 1, false)];
    let stale = [ChatLineInput {
        seq: 3,
        text: "I can't reach that!",
        type_: 0,
        username: None,
    }];
    let mut snap = base(stand());
    snap.npcs = &npcs;
    snap.chat_continue = true;
    snap.chat_lines = &stale;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts().len(), 1);
    snap.tick = 2;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn fresh_cant_reach_reachable_adj_is_unreachable_without_clear() {
    let iso = spawn(NPC_DIALOG);
    let actions = ["Talk-to".to_string()];
    let npcs = [npc("Traiborn", &actions, 4, 8, 5, 2, true)];
    let mut snap = base(stand());
    snap.npcs = &npcs;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts().len(), 1);
    let fresh = [ChatLineInput {
        seq: 8,
        text: "I can't reach that!",
        type_: 0,
        username: None,
    }];
    snap.tick = 2;
    snap.chat_lines = &fresh;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.probe("__ok").unwrap(), "unreachable");
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn one_clear_walk_then_second_cant_reach_is_unreachable() {
    let iso = spawn(NPC_DIALOG);
    let actions = ["Talk-to".to_string()];
    let npcs = [npc("Traiborn", &actions, 4, 8, 5, 2, false)];
    let mut snap = base(stand());
    snap.npcs = &npcs;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Npc {
            name: "Traiborn".into(),
            action: "Talk-to".into(),
            index: Some(4),
        }]
    );
    let first = [ChatLineInput {
        seq: 8,
        text: "I can't reach that!",
        type_: 0,
        username: None,
    }];
    snap.tick = 2;
    snap.chat_lines = &first;
    post(&iso, &snap);
    tick(&iso, 2);
    match &iso.drain_interacts()[..] {
        [InteractReq::WalkNear {
            x: 8,
            z: 5,
            radius: 1,
            request_id,
            ..
        }] => assert_ne!(*request_id, 0),
        other => panic!("expected one Clear walk-near, got {other:?}"),
    }
    snap.tick = 3;
    snap.here = Some(TileInput {
        x: 8,
        z: 5,
        level: 0,
    });
    post(&iso, &snap);
    tick(&iso, 3);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Npc {
            name: "Traiborn".into(),
            action: "Talk-to".into(),
            index: Some(4),
        }]
    );
    let second = [ChatLineInput {
        seq: 9,
        text: "I can't reach that!",
        type_: 0,
        username: None,
    }];
    snap.tick = 4;
    snap.chat_lines = &second;
    post(&iso, &snap);
    tick(&iso, 4);
    assert_eq!(iso.probe("__ok").unwrap(), "unreachable");
    assert!(iso.drain_interacts().is_empty(), "no second Clear");
    iso.join();
}

#[test]
fn clear_correlated_fail_is_unreachable_without_another_npc() {
    let iso = spawn(NPC_DIALOG);
    let actions = ["Talk-to".to_string()];
    let npcs = [npc("Traiborn", &actions, 4, 8, 5, 2, false)];
    let mut snap = base(stand());
    snap.npcs = &npcs;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts().len(), 1);
    let first = [ChatLineInput {
        seq: 8,
        text: "I can't reach that!",
        type_: 0,
        username: None,
    }];
    snap.tick = 2;
    snap.chat_lines = &first;
    post(&iso, &snap);
    tick(&iso, 2);
    let request_id = match &iso.drain_interacts()[..] {
        [InteractReq::WalkNear {
            x: 8,
            z: 5,
            radius: 1,
            request_id,
            ..
        }] => *request_id,
        other => panic!("expected Clear walk-near, got {other:?}"),
    };
    snap.tick = 3;
    post_native(&iso, &snap, fail_native(request_id, 8, 5, 1));
    tick(&iso, 3);
    assert_eq!(iso.probe("__ok").unwrap(), "unreachable");
    assert!(
        iso.drain_interacts().is_empty(),
        "Clear correlated fail must not emit another npc"
    );
    iso.join();
}

#[test]
fn nearest_same_name_talk_uses_posted_index() {
    let iso = spawn(NPC_DIALOG);
    let actions = ["Talk-to".to_string()];
    let npcs = [
        npc("Traiborn", &actions, 2, 9, 5, 4, true),
        npc("Traiborn", &actions, 11, 6, 5, 1, true),
    ];
    let mut snap = base(stand());
    snap.npcs = &npcs;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Npc {
            name: "Traiborn".into(),
            action: "Talk-to".into(),
            index: Some(11),
        }]
    );
    iso.join();
}

#[test]
fn open_ms_zero_retries_without_continue_or_answer() {
    let iso = spawn(
        r#"
import { Reach } from '../../api/walking/Reach.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = await Reach.npcDialog({
            name: 'Traiborn',
            near: { x: 5, z: 5, level: 0 },
            openMs: 0,
        });
    }
}
"#,
    );
    let actions = ["Talk-to".to_string()];
    let npcs = [npc("Traiborn", &actions, 4, 6, 5, 1, true)];
    let mut snap = base(stand());
    snap.npcs = &npcs;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts().len(), 1);
    snap.tick = 2;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.probe("__ok").unwrap(), "retry");
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn hold_and_reset_do_not_emit_another_talk() {
    let iso = spawn(NPC_DIALOG);
    let actions = ["Talk-to".to_string()];
    let npcs = [npc("Traiborn", &actions, 4, 6, 5, 1, true)];
    let mut snap = base(stand());
    snap.npcs = &npcs;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts().len(), 1);

    iso.pause();
    snap.tick = 2;
    snap.chat_modal_id = 241;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);
    assert!(iso.drain_interacts().is_empty());
    iso.resume();

    snap.hold = true;
    snap.chat_modal_id = -1;
    snap.tick = 3;
    post(&iso, &snap);
    tick(&iso, 3);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);
    assert!(iso.drain_interacts().is_empty());

    snap.hold = false;
    snap.tick = 4;
    post(&iso, &snap);
    tick(&iso, 4);
    assert_eq!(iso.probe("__ok").unwrap(), "retry");
    assert!(iso.drain_interacts().is_empty());

    iso.reset_session_work();
    snap.tick = 5;
    post(&iso, &snap);
    tick(&iso, 5);
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

const ENTITY_OP: &str = r#"
import { Reach } from '../../api/walking/Reach.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        globalThis.__logs = [];
        globalThis.__clicks = 0;
        const entity = {
            interact: (op) => { globalThis.__clicks++; globalThis.__op = op; return true; },
            tile: () => ({ x: 8, z: 5, level: 0 }),
        };
        globalThis.__ok = await Reach.entityOp({
            find: () => (globalThis.__gone ? null : entity),
            op: 'Attack',
            expect: () => globalThis.__ready === true,
            log: (m) => globalThis.__logs.push(String(m)),
        });
    }
}
"#;

#[test]
fn entity_op_clicks_once_then_settles_done_on_expect() {
    let iso = spawn(ENTITY_OP);
    let mut snap = base(stand());
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__clicks").unwrap(), 1);
    assert_eq!(iso.probe("__op").unwrap(), "Attack");
    snap.tick = 2;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Null,
        "still waiting on expect"
    );
    iso.probe("globalThis.__ready = true").unwrap();
    snap.tick = 3;
    post(&iso, &snap);
    tick(&iso, 3);
    tick(&iso, 4);
    assert_eq!(iso.probe("__ok").unwrap(), "done");
    assert_eq!(iso.probe("__clicks").unwrap(), 1, "no re-click");
    iso.join();
}

#[test]
fn entity_op_cant_reach_without_a_door_is_unreachable_with_the_frozen_log() {
    let iso = spawn(ENTITY_OP);
    let mut snap = base(stand());
    post(&iso, &snap);
    tick(&iso, 1);
    let lines = [ChatLineInput {
        seq: 3,
        text: "I can't reach that!",
        type_: 0,
        username: None,
    }];
    snap.tick = 2;
    snap.chat_lines = &lines;
    post(&iso, &snap);
    tick(&iso, 2);
    tick(&iso, 3);
    assert_eq!(iso.probe("__ok").unwrap(), "unreachable");
    assert_eq!(
        iso.probe("__logs").unwrap(),
        serde_json::json!([
            "reach: 'Attack' at (8,5): server can't reach it and no door in front to open or close (unreachable)"
        ])
    );
    iso.join();
}

#[test]
fn entity_op_that_could_not_click_retries_after_one_tick() {
    let iso = spawn(ENTITY_OP);
    iso.probe("globalThis.__gone = true").unwrap();
    let snap = base(stand());
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);
    tick(&iso, 2);
    tick(&iso, 3);
    assert_eq!(iso.probe("__ok").unwrap(), "retry");
    assert_eq!(iso.probe("__clicks").unwrap(), 0);
    iso.join();
}
