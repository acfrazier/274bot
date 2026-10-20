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
        shape: 0,
        angle: 0,
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
        user_move_intent_seq: 0,
        walk_outcome_cancel_reason: Default::default(),
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
fn missing_npc_close_in_failure_keeps_the_resilient_ladder_going() {
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
            compat_tile_intent: true,
            allow_teleports: false,
            allow_wilderness: true,
            allow_bank_fetch: true,
            request_id,
            avoid: _,
            cross: _,
        }] => {
            assert_ne!(*request_id, 0);
            *request_id
        }
        other => panic!("expected close-in walk-near, got {other:?}"),
    };
    snap.tick = 2;
    post_native(&iso, &snap, fail_native(request_id, 5, 5, 3));
    tick(&iso, 2);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Null,
        "frozen closeIn is a walkResilient ladder: one failed walk does not end it"
    );
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::WalkTo {
            x: 5,
            z: 5,
            level: 0
        }],
        "the ladder's scene step follows the failed baked walk"
    );
    iso.join();
}

#[test]
fn stand_walk_failure_takes_the_ladder_scene_step_before_the_talk() {
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
        vec![InteractReq::WalkTo {
            x: 5,
            z: 5,
            level: 0
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
fn open_ms_zero_talks_again_next_round_instead_of_settling() {
    // Frozen npcDialog passes `retryAfterTimeout: true` (Reach.ts:288-300):
    // an unanswered talk waits one tick and clicks again, it does not
    // settle `retry` after one window.
    let iso = spawn(NPC_DIALOG);
    iso.probe("globalThis.__openMs = 0").unwrap();
    let actions = ["Talk-to".to_string()];
    let npcs = [npc("Traiborn", &actions, 4, 6, 5, 1, true)];
    let mut snap = base(stand());
    snap.npcs = &npcs;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts().len(), 1);
    for n in 2..=3 {
        snap.tick = n;
        post(&iso, &snap);
        tick(&iso, n);
    }
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Npc {
            name: "Traiborn".into(),
            action: "Talk-to".into(),
            index: Some(4),
        }],
        "the next round talks again"
    );
    iso.join();
}

#[test]
fn npc_dialog_opens_the_door_in_front_of_an_unreachable_npc_then_talks() {
    // Frozen probeUnreachable (Reach.ts:167-176): the NPC at (8,5) is out
    // of reach behind the door at (7,5), so the reach walks to the door and
    // opens it before its first click, then talks.
    let iso = spawn(NPC_DIALOG);
    let open = ["Open".to_string()];
    let talk = ["Talk-to".to_string()];
    let g = grid((5, 5), &[(6, 5)], &[]);
    let with_door = [barrier("Door", &open, 1530, 7, 5, 2)];
    let npcs = [npc("Traiborn", &talk, 4, 8, 5, 3, false)];
    let mut snap = base(stand());
    snap.reach = view(&g);
    snap.locs = &with_door;
    snap.npcs = &npcs;
    post(&iso, &snap);
    tick(&iso, 1);
    let request_id = match iso.drain_interacts().as_slice() {
        [InteractReq::WalkNear {
            x: 7,
            z: 5,
            radius: 1,
            request_id,
            ..
        }] => *request_id,
        other => panic!("walk to the blocking door first, got {other:?}"),
    };
    // Frozen approaches the door with walkResilient (Reach.ts:112-114): a
    // failed baked walk takes the ladder's scene step, not the Open.
    snap.tick = 2;
    post_native(&iso, &snap, fail_native(request_id, 7, 5, 1));
    tick(&iso, 2);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::WalkTo {
            x: 7,
            z: 5,
            level: 0
        }]
    );
    let beside = grid((6, 5), &[], &[(7, 5)]);
    snap.tick = 3;
    snap.here = Some(TileInput {
        x: 6,
        z: 5,
        level: 0,
    });
    snap.reach = view(&beside);
    post(&iso, &snap);
    tick(&iso, 3);
    assert_eq!(iso.drain_interacts(), vec![loc_op(7, 5, "Open", 1530)]);

    snap.tick = 4;
    snap.locs = &[];
    post(&iso, &snap);
    tick(&iso, 4);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Npc {
            name: "Traiborn".into(),
            action: "Talk-to".into(),
            index: Some(4),
        }],
        "the cleared round talks"
    );
    snap.tick = 5;
    snap.chat_modal_id = 241;
    post(&iso, &snap);
    tick(&iso, 5);
    assert_eq!(iso.probe("__ok").unwrap(), "done");
    assert!(logs(&iso).contains(&"reach: opening blocking 'Door' at (7,5)".to_string()));
    iso.join();
}

/// The door-approach walk of an interrupted reach is stopped: a guardian
/// hold only freezes the host follow, which would resume after release.
#[test]
fn a_hold_during_the_door_approach_stops_the_armed_walk() {
    let iso = spawn(NPC_DIALOG);
    let open = ["Open".to_string()];
    let talk = ["Talk-to".to_string()];
    let g = grid((5, 5), &[(6, 5)], &[]);
    let with_door = [barrier("Door", &open, 1530, 7, 5, 2)];
    let npcs = [npc("Traiborn", &talk, 4, 8, 5, 3, false)];
    let mut snap = base(stand());
    snap.reach = view(&g);
    snap.locs = &with_door;
    snap.npcs = &npcs;
    post(&iso, &snap);
    tick(&iso, 1);
    let request_id = match iso.drain_interacts().as_slice() {
        [InteractReq::WalkNear {
            x: 7,
            z: 5,
            request_id,
            ..
        }] => *request_id,
        other => panic!("walk to the blocking door first, got {other:?}"),
    };
    snap.tick = 2;
    snap.hold = true;
    post(&iso, &snap);
    tick(&iso, 2);
    assert!(iso.drain_interacts().is_empty(), "held rows do not step");
    snap.tick = 3;
    snap.hold = false;
    post(&iso, &snap);
    tick(&iso, 3);
    assert_eq!(iso.probe("__ok").unwrap(), "retry");
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::AbortWalk { request_id }],
        "the interrupted reach stops its host walk"
    );
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

    iso.reset_session_work(None);
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
            interact: (op) => {
                globalThis.__clicks++;
                globalThis.__op = op;
                return globalThis.__asyncClick ? Promise.resolve(false) : true;
            },
            tile: () => ({ x: 8, z: 5, level: 0 }),
        };
        globalThis.__ok = await Reach.entityOp({
            find: () => (globalThis.__gone ? null : entity),
            op: 'Attack',
            expect: () => (globalThis.__asyncExpect
                ? Promise.resolve(false)
                : globalThis.__ready === true),
            log: (m) => globalThis.__logs.push(String(m)),
            openWhenUnreachable: globalThis.__probe === true,
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
fn entity_op_cant_reach_without_a_door_is_unreachable() {
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

const GRID: i32 = 16;

/// A posted reach view over `0..16 x 0..16` on level 0. `walkable` is
/// scene geometry; `reachable` and `reachable_adj` are the flood from `here`.
struct Grid {
    walkable: Vec<u32>,
    reachable: Vec<u32>,
    reachable_adj: Vec<u32>,
    exact_rank: Vec<u16>,
    adjacent_rank: Vec<u16>,
    step: Vec<u8>,
}

fn grid(here: (i32, i32), exact: &[(i32, i32)], adj: &[(i32, i32)]) -> Grid {
    let n = (GRID * GRID) as usize;
    let index = |(x, z): (i32, i32)| (x * GRID + z) as usize;
    let mut g = Grid {
        walkable: vec![0; n.div_ceil(32)],
        reachable: vec![0; n.div_ceil(32)],
        reachable_adj: vec![0; n.div_ceil(32)],
        exact_rank: vec![u16::MAX; n],
        adjacent_rank: vec![u16::MAX; n],
        step: vec![0; n],
    };
    for &tile in exact.iter().chain([&here]) {
        let i = index(tile);
        g.walkable[i / 32] |= 1 << (i % 32);
        g.reachable[i / 32] |= 1 << (i % 32);
        g.exact_rank[i] = if tile == here { 0 } else { 1 };
    }
    for &tile in adj.iter().chain(exact).chain([&here]) {
        let i = index(tile);
        g.reachable_adj[i / 32] |= 1 << (i % 32);
        g.adjacent_rank[i] = if tile == here { 0 } else { 1 };
    }
    g
}

fn allow_walkable(g: &mut Grid, tiles: &[(i32, i32)]) {
    for &(x, z) in tiles {
        let i = (x * GRID + z) as usize;
        g.walkable[i / 32] |= 1 << (i % 32);
    }
}

fn view(g: &Grid) -> ReachViewInput<'_> {
    ReachViewInput {
        available: true,
        base_x: 0,
        base_z: 0,
        level: 0,
        width: GRID,
        height: GRID,
        walkable: &g.walkable,
        reachable: &g.reachable,
        reachable_adj: &g.reachable_adj,
        exact_rank: &g.exact_rank,
        adjacent_rank: &g.adjacent_rank,
        step: &g.step,
        canlight: &[],
        stamp: 0,
    }
}

fn barrier<'a>(
    name: &'a str,
    actions: &'a [String],
    id: i32,
    x: i32,
    z: i32,
    distance: i32,
) -> SceneEntityInput<'a> {
    let mut loc = npc(name, actions, 0, x, z, distance, false);
    loc.id = id;
    loc
}

fn loc_op(x: i32, z: i32, action: &str, id: i32) -> InteractReq {
    InteractReq::Loc {
        x,
        z,
        level: 0,
        action: action.into(),
        id: Some(id),
    }
}

fn cant_reach(seq: i32) -> [ChatLineInput<'static>; 1] {
    [ChatLineInput {
        seq,
        text: "I can't reach that!",
        type_: 0,
        username: None,
    }]
}

fn logs(iso: &LoadIsolate) -> Vec<String> {
    serde_json::from_value(iso.probe("__logs").unwrap()).unwrap()
}

#[test]
fn entity_op_walks_to_and_opens_the_door_toward_the_target_then_retries() {
    let iso = spawn(ENTITY_OP);
    let open = ["Open".to_string()];
    // (6,5) is reachable, the target (8,5) is not. The decoy is nearer but
    // lies away from the target (frozen towardDest), so it is not opened.
    let g = grid((5, 5), &[(6, 5), (1, 1)], &[]);
    let locs = [
        barrier("Door", &open, 1530, 7, 5, 2),
        barrier("Door", &open, 99, 0, 1, 1),
    ];
    let mut snap = base(stand());
    snap.reach = view(&g);
    snap.locs = &locs;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__clicks").unwrap(), 1);

    let lines = cant_reach(3);
    snap.tick = 2;
    snap.chat_lines = &lines;
    post(&iso, &snap);
    tick(&iso, 2);
    let walk = iso.drain_interacts();
    let request_id = match walk.as_slice() {
        [InteractReq::WalkNear {
            x: 7,
            z: 5,
            radius: 1,
            request_id,
            ..
        }] => *request_id,
        other => panic!("walk to the blocking door, got {other:?}"),
    };

    // The approach is frozen walkResilient (Reach.ts:112-114): a failed
    // baked walk takes the scene step; the Open follows once the ladder
    // ends beside the door.
    snap.tick = 3;
    post_native(&iso, &snap, fail_native(request_id, 7, 5, 1));
    tick(&iso, 3);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::WalkTo {
            x: 7,
            z: 5,
            level: 0
        }]
    );
    let beside = grid((6, 5), &[], &[(7, 5)]);
    snap.here = Some(TileInput {
        x: 6,
        z: 5,
        level: 0,
    });
    snap.reach = view(&beside);
    post(&iso, &snap);
    tick(&iso, 3);
    assert_eq!(iso.drain_interacts(), vec![loc_op(7, 5, "Open", 1530)]);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);

    let decoy_only = [barrier("Door", &open, 99, 0, 1, 1)];
    snap.tick = 4;
    snap.locs = &decoy_only;
    post(&iso, &snap);
    tick(&iso, 4);
    assert_eq!(
        iso.probe("__clicks").unwrap(),
        2,
        "the next round clicks again"
    );

    iso.probe("globalThis.__ready = true").unwrap();
    snap.tick = 5;
    post(&iso, &snap);
    tick(&iso, 5);
    tick(&iso, 6);
    assert_eq!(iso.probe("__ok").unwrap(), "done");
    assert_eq!(logs(&iso), vec!["reach: opening blocking 'Door' at (7,5)"]);
    iso.join();
}

#[test]
fn entity_op_walks_through_an_open_leaf_without_closing_it() {
    let iso = spawn(ENTITY_OP);
    let close = ["Close".to_string()];
    let g = grid((5, 5), &[], &[]);
    let locs = [barrier("Door", &close, 1531, 8, 4, 3)];
    let mut snap = base(stand());
    snap.reach = view(&g);
    snap.locs = &locs;
    post(&iso, &snap);
    tick(&iso, 1);
    let lines = cant_reach(3);
    snap.tick = 2;
    snap.chat_lines = &lines;
    post(&iso, &snap);
    tick(&iso, 2);
    assert!(
        matches!(
            iso.drain_interacts().as_slice(),
            [InteractReq::WalkNear {
                x: 8,
                z: 5,
                radius: 1,
                ..
            }]
        ),
        "recovery walks toward the target through the open passage, never Close"
    );
    assert_eq!(iso.probe("__clicks").unwrap(), 1);

    let reached = grid((8, 4), &[], &[(8, 5)]);
    snap.here = Some(TileInput {
        x: 8,
        z: 4,
        level: 0,
    });
    snap.reach = view(&reached);
    post(&iso, &snap);
    tick(&iso, 3);
    assert_eq!(iso.probe("__clicks").unwrap(), 2);
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn west_straight_wall_door_opens_from_engine_reachable_side_without_walk() {
    let iso = spawn(NPC_DIALOG);
    let open = ["Open".to_string()];
    let talk = ["Talk-to".to_string()];
    let mut g = grid((5, 5), &[], &[]);
    allow_walkable(&mut g, &[(6, 5)]);
    let mut door = barrier("Door", &open, 1530, 6, 5, 1);
    door.shape = 0;
    door.angle = 0; // WALL_STRAIGHT / WEST.
    let doors = [door];
    let npcs = [npc("Traiborn", &talk, 4, 8, 5, 3, false)];
    let mut snap = base(stand());
    snap.reach = view(&g);
    snap.locs = &doors;
    snap.npcs = &npcs;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![loc_op(6, 5, "Open", 1530)],
        "the engine-reachable west stand opens directly instead of walking across the wall"
    );
    iso.join();
}

#[test]
fn non_straight_wall_door_keeps_the_approach_walk() {
    let iso = spawn(NPC_DIALOG);
    let open = ["Open".to_string()];
    let talk = ["Talk-to".to_string()];
    let mut g = grid((5, 5), &[], &[]);
    allow_walkable(&mut g, &[(6, 5)]);
    let mut door = barrier("Door", &open, 1530, 6, 5, 1);
    door.shape = 9;
    door.angle = 0;
    let doors = [door];
    let npcs = [npc("Traiborn", &talk, 4, 8, 5, 3, false)];
    let mut snap = base(stand());
    snap.reach = view(&g);
    snap.locs = &doors;
    snap.npcs = &npcs;
    post(&iso, &snap);
    tick(&iso, 1);
    assert!(
        matches!(
            iso.drain_interacts().as_slice(),
            [InteractReq::WalkNear {
                x: 6,
                z: 5,
                radius: 1,
                ..
            }]
        ),
        "non-straight wall geometry must not inherit the engine's straight-wall shortcut"
    );
    iso.join();
}

#[test]
fn wall_aware_hop_finish_does_not_settle_across_a_wall() {
    let iso = spawn(
        r#"
import { walkWithHops } from '../../api/ai/quests/exec/primitives.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        globalThis.__ok = await walkWithHops({ x: 8, z: 5, level: 0 }, 3, [], () => {});
    }
}
"#,
    );
    let blocked = grid((5, 5), &[], &[]);
    let mut snap = base(stand());
    snap.reach = view(&blocked);
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Null,
        "a radius disk across a wall is not arrival"
    );
    assert!(matches!(
        iso.drain_interacts().as_slice(),
        [InteractReq::WalkNear {
            x: 8,
            z: 5,
            radius: 3,
            ..
        }]
    ));

    let reached = grid((7, 5), &[(8, 5)], &[]);
    snap.tick = 2;
    snap.here = Some(TileInput {
        x: 7,
        z: 5,
        level: 0,
    });
    snap.reach = view(&reached);
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.probe("__ok").unwrap(), true);
    iso.join();
}

#[test]
fn open_when_unreachable_probes_and_clears_before_the_first_click() {
    let iso = spawn(ENTITY_OP);
    iso.probe("globalThis.__probe = true").unwrap();
    let open = ["Open".to_string()];
    let g = grid((5, 5), &[], &[(6, 5)]);
    let locs = [barrier("Door", &open, 1530, 6, 5, 1)];
    let mut snap = base(stand());
    snap.reach = view(&g);
    snap.locs = &locs;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.probe("__clicks").unwrap(),
        0,
        "the scene probe clears the way before any click"
    );
    assert_eq!(iso.drain_interacts(), vec![loc_op(6, 5, "Open", 1530)]);

    let reached = grid((5, 5), &[], &[(8, 5)]);
    snap.tick = 2;
    snap.reach = view(&reached);
    snap.locs = &[];
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(
        iso.probe("__clicks").unwrap(),
        1,
        "reachable now: one click"
    );
    assert!(iso.drain_interacts().is_empty(), "no second clear");
    iso.join();
}

#[test]
fn entity_op_gives_up_retry_after_eight_cleared_rounds() {
    let iso = spawn(ENTITY_OP);
    let open = ["Open".to_string()];
    let g = grid((5, 5), &[], &[(6, 5)]);
    let door = [barrier("Door", &open, 1530, 6, 5, 1)];
    let mut snap = base(stand());
    snap.reach = view(&g);
    snap.locs = &door;
    post(&iso, &snap);
    tick(&iso, 1);
    let lines: Vec<_> = (0..8).map(|round| cant_reach(10 + round)).collect();
    let mut opens = 0;
    let mut n = 1;
    // Each round: a fresh "can't reach" opens the adjacent door; the door
    // is gone next tick, the clear succeeds and the next round clicks.
    for lines in &lines {
        n += 1;
        snap.tick = n;
        snap.locs = &door;
        snap.chat_lines = lines;
        post(&iso, &snap);
        tick(&iso, n);
        if iso.drain_interacts() == vec![loc_op(6, 5, "Open", 1530)] {
            opens += 1;
        }
        n += 1;
        snap.tick = n;
        snap.locs = &[];
        post(&iso, &snap);
        tick(&iso, n);
    }
    assert_eq!(opens, 8);
    assert_eq!(
        iso.probe("__clicks").unwrap(),
        8,
        "one click per round, none after the eighth clear"
    );
    assert_eq!(iso.probe("__ok").unwrap(), "retry");
    iso.join();
}

#[test]
fn entity_op_calls_expect_synchronously_but_awaits_the_click() {
    // Frozen `if (opts.expect())`: a promise is truthy, so the op counts
    // as dispatched and done without a click.
    let iso = spawn(ENTITY_OP);
    iso.probe("globalThis.__asyncExpect = true").unwrap();
    post(&iso, &base(stand()));
    tick(&iso, 1);
    tick(&iso, 2);
    tick(&iso, 3);
    assert_eq!(iso.probe("__ok").unwrap(), "done");
    assert_eq!(iso.probe("__clicks").unwrap(), 0);
    iso.join();

    // Frozen `await entity.interact(op)`: a click that resolves false did
    // not dispatch, so the call retries instead of watching `expect`.
    let iso = spawn(ENTITY_OP);
    iso.probe("globalThis.__asyncClick = true").unwrap();
    post(&iso, &base(stand()));
    tick(&iso, 1);
    tick(&iso, 2);
    tick(&iso, 3);
    assert_eq!(iso.probe("__clicks").unwrap(), 1);
    assert_eq!(iso.probe("__ok").unwrap(), "retry");
    iso.join();
}

const LOC_OP: &str = r#"
import { Reach } from '../../api/walking/Reach.js';
import { Sustain } from '../../api/sustain/Sustain.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        globalThis.__logs = [];
        globalThis.__fed = 0;
        Sustain.set(() => { globalThis.__fed += 1; });
        try {
            globalThis.__ok = await Reach.locOp({
                name: 'Barrel of flour',
                id: 7000,
                op: 'Take From',
                near: globalThis.__near || { x: 5, z: 5, level: 0 },
                within: 6,
                expectMs: '__expectMs' in globalThis ? globalThis.__expectMs : 1200,
                expect: () => globalThis.__ready === true,
                refused: /guards' permission|something to put the flour in/i,
                log: (m) => globalThis.__logs.push(String(m)),
            });
        } catch (e) {
            globalThis.__ok = String(e.message || e);
        }
    }
}
"#;

/// The posted `Loc` clicks in `ops`.
fn loc_clicks(ops: &[InteractReq]) -> usize {
    ops.iter()
        .filter(|op| matches!(op, InteractReq::Loc { .. }))
        .count()
}

/// Tick from `from` with `pause` between ticks until `__ok` settles or
/// `ticks` ran out. Returns every op the ticks emitted and the next tick.
fn tick_until_settled(
    iso: &LoadIsolate,
    from: u64,
    ticks: u64,
    pause: std::time::Duration,
) -> (Vec<InteractReq>, u64) {
    let mut ops = Vec::new();
    let mut n = from;
    // `__ok` exists only once the script's first loop has run.
    while n < from + ticks && iso.probe("globalThis.__ok ?? null").unwrap() == Value::Null {
        std::thread::sleep(pause);
        tick(iso, n);
        ops.extend(iso.drain_interacts());
        n += 1;
    }
    (ops, n)
}

fn barrel<'a>(actions: &'a [String], x: i32, z: i32, distance: i32) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index: 0,
        id: 7000,
        name: Some("Barrel of flour"),
        x,
        z,
        level: 0,
        distance,
        health: 0,
        max_health: 0,
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

#[test]
fn loc_op_clicks_the_named_loc_then_settles_done_on_expect() {
    let iso = spawn(LOC_OP);
    let actions = ["Take From".to_string(), "Examine".to_string()];
    let locs = [barrel(&actions, 6, 5, 1)];
    let mut snap = base(stand());
    snap.locs = &locs;
    post(&iso, &snap);
    tick(&iso, 1);
    // The first doors round runs on a later pump than the begin.
    tick(&iso, 2);
    assert_eq!(
        iso.drain_interacts(),
        vec![loc_op(6, 5, "Take From", 7000)],
        "one click on the exact loc row"
    );
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null, "waiting on expect");
    iso.probe("globalThis.__ready = true").unwrap();
    snap.tick = 3;
    post(&iso, &snap);
    tick(&iso, 3);
    tick(&iso, 4);
    tick(&iso, 5);
    assert_eq!(iso.probe("__ok").unwrap(), "done");
    assert!(iso.drain_interacts().is_empty(), "no re-click");
    iso.join();
}

#[test]
fn loc_op_refused_message_is_retry_without_a_second_click() {
    let iso = spawn(LOC_OP);
    let actions = ["Take From".to_string()];
    let locs = [barrel(&actions, 6, 5, 1)];
    let mut snap = base(stand());
    snap.locs = &locs;
    post(&iso, &snap);
    tick(&iso, 1);
    tick(&iso, 2);
    assert_eq!(iso.drain_interacts(), vec![loc_op(6, 5, "Take From", 7000)]);
    let lines = [ChatLineInput {
        seq: 4,
        text: "You need the Guards' permission to do that.",
        type_: 0,
        username: None,
    }];
    snap.tick = 3;
    snap.chat_lines = &lines;
    post(&iso, &snap);
    for n in 3..=7 {
        tick(&iso, n);
    }
    assert_eq!(iso.probe("__ok").unwrap(), "retry");
    assert!(
        iso.drain_interacts().is_empty(),
        "a refused op is not re-clicked"
    );
    let logs = iso.probe("__logs.join('|')").unwrap();
    assert_eq!(
        logs,
        "reach: 'Barrel of flour' refused the op \u{2014} not retrying"
    );
    iso.join();
}

#[test]
fn loc_op_missing_loc_far_from_the_stand_closes_in_instead_of_clicking() {
    let iso = spawn(LOC_OP);
    iso.probe("globalThis.__near = { x: 20, z: 20, level: 0 }")
        .unwrap();
    let mut snap = base(TileInput {
        x: 0,
        z: 0,
        level: 0,
    });
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);
    match &iso.drain_interacts()[..] {
        [InteractReq::WalkNear {
            x: 20,
            z: 20,
            level: 0,
            radius: 2,
            ..
        }] => {}
        other => panic!("expected the close-in walk-near at radius 2, got {other:?}"),
    }
    snap.tick = 2;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Null,
        "the close-in ladder is still walking"
    );
    iso.join();
}

#[test]
fn loc_op_ignores_a_same_name_loc_without_the_op() {
    let iso = spawn(LOC_OP);
    // Present in scene, so no close-in; but it has no 'Take From', so the
    // find the click rounds use never matches and nothing is clicked.
    let actions = ["Examine".to_string()];
    let locs = [barrel(&actions, 6, 5, 1)];
    let mut snap = base(stand());
    snap.locs = &locs;
    post(&iso, &snap);
    for n in 1..=6 {
        tick(&iso, n);
    }
    assert!(
        iso.drain_interacts().is_empty(),
        "a loc without the op is never clicked"
    );
    // Frozen `sceneSettled` gives up after 3 s, then `closeIn` from the
    // stand itself has nothing to walk: `'retry'` (`Reach.ts:61-82`).
    std::thread::sleep(std::time::Duration::from_millis(3_100));
    tick(&iso, 7);
    tick(&iso, 8);
    assert_eq!(iso.probe("__ok").unwrap(), "retry");
    assert!(
        iso.drain_interacts().is_empty(),
        "nothing clicked or walked"
    );
    iso.join();
}

/// The `GameMessages` history counts its own sequence from 1, so a refusal
/// mark taken from the host chat `seq` (50 here) would never see the
/// refusal at host seq 51 (FIX-024-COMPAT-MISC review MAJ-1).
#[test]
fn loc_op_refused_after_an_earlier_message_is_retry_without_a_second_click() {
    let iso = spawn(LOC_OP);
    let actions = ["Take From".to_string()];
    let locs = [barrel(&actions, 6, 5, 1)];
    let mut snap = base(stand());
    snap.locs = &locs;
    let before = [ChatLineInput {
        seq: 50,
        text: "Welcome to RuneScape.",
        type_: 0,
        username: None,
    }];
    snap.chat_lines = &before;
    post(&iso, &snap);
    tick(&iso, 1);
    tick(&iso, 2);
    assert_eq!(iso.drain_interacts(), vec![loc_op(6, 5, "Take From", 7000)]);
    let lines = [
        ChatLineInput {
            seq: 51,
            text: "You need the Guards' permission to do that.",
            type_: 0,
            username: None,
        },
        ChatLineInput {
            seq: 50,
            text: "Welcome to RuneScape.",
            type_: 0,
            username: None,
        },
    ];
    snap.tick = 3;
    snap.chat_lines = &lines;
    post(&iso, &snap);
    for n in 3..=7 {
        tick(&iso, n);
    }
    assert_eq!(iso.probe("__ok").unwrap(), "retry");
    let clicks = iso.drain_interacts();
    // Past `expectMs: 1200`: a missed refusal would click again here.
    std::thread::sleep(std::time::Duration::from_millis(1_500));
    for n in 8..=12 {
        snap.tick = n;
        post(&iso, &snap);
        tick(&iso, n);
    }
    let later = iso.drain_interacts();
    assert!(
        clicks.is_empty() && later.is_empty(),
        "a refused op is not re-clicked"
    );
    assert_eq!(
        iso.probe("__logs.join('|')").unwrap(),
        "reach: 'Barrel of flour' refused the op \u{2014} not retrying"
    );
    iso.join();
}

/// Frozen `locOp` passes `retryAfterTimeout: true` (`Reach.ts:257-265`): an
/// unanswered click waits one tick and clicks again, it does not settle.
#[test]
fn loc_op_times_out_then_clicks_again_on_the_next_round() {
    let iso = spawn(LOC_OP);
    iso.probe("globalThis.__expectMs = 50").unwrap();
    let actions = ["Take From".to_string()];
    let locs = [barrel(&actions, 6, 5, 1)];
    let mut snap = base(stand());
    snap.locs = &locs;
    post(&iso, &snap);
    tick(&iso, 1);
    tick(&iso, 2);
    assert_eq!(iso.drain_interacts(), vec![loc_op(6, 5, "Take From", 7000)]);
    std::thread::sleep(std::time::Duration::from_millis(80));
    let mut clicks = Vec::new();
    for n in 3..=6 {
        tick(&iso, n);
        clicks.extend(iso.drain_interacts());
    }
    assert_eq!(clicks, vec![loc_op(6, 5, "Take From", 7000)]);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null, "still reaching");
    iso.join();
}

/// Frozen `REACH_DOOR_ATTEMPTS` (`Reach.ts:84,167`): eight timed-out rounds,
/// one click each, then `'retry'`.
#[test]
fn loc_op_gives_up_retry_after_eight_timed_out_rounds() {
    let iso = spawn(LOC_OP);
    iso.probe("globalThis.__expectMs = 1").unwrap();
    let actions = ["Take From".to_string()];
    let locs = [barrel(&actions, 6, 5, 1)];
    let mut snap = base(stand());
    snap.locs = &locs;
    post(&iso, &snap);
    let (ops, _) = tick_until_settled(&iso, 1, 60, std::time::Duration::from_millis(3));
    assert_eq!(loc_clicks(&ops), 8, "{ops:?}");
    assert_eq!(iso.probe("__ok").unwrap(), "retry");
    iso.join();
}

/// Frozen `reachThroughDoors` (`Reach.ts:193-207`): "I can't reach that!"
/// opens the door toward the loc, then the next round clicks again.
#[test]
fn loc_op_cant_reach_opens_the_door_toward_the_loc_then_clicks_again() {
    let iso = spawn(LOC_OP);
    iso.probe("globalThis.__near = { x: 6, z: 5, level: 0 }")
        .unwrap();
    let open = ["Open".to_string()];
    let take = ["Take From".to_string()];
    let beside = grid((6, 5), &[], &[(7, 5)]);
    let locs = [
        barrel(&take, 8, 5, 2),
        barrier("Door", &open, 1530, 7, 5, 1),
    ];
    let mut snap = base(TileInput {
        x: 6,
        z: 5,
        level: 0,
    });
    snap.reach = view(&beside);
    snap.locs = &locs;
    post(&iso, &snap);
    tick(&iso, 1);
    tick(&iso, 2);
    assert_eq!(iso.drain_interacts(), vec![loc_op(8, 5, "Take From", 7000)]);

    let lines = cant_reach(3);
    snap.tick = 3;
    snap.chat_lines = &lines;
    post(&iso, &snap);
    tick(&iso, 3);
    tick(&iso, 4);
    assert_eq!(iso.drain_interacts(), vec![loc_op(7, 5, "Open", 1530)]);

    let open_now = [barrel(&take, 8, 5, 2)];
    snap.tick = 5;
    snap.locs = &open_now;
    post(&iso, &snap);
    let mut clicks = Vec::new();
    for n in 5..=7 {
        tick(&iso, n);
        clicks.extend(iso.drain_interacts());
    }
    assert_eq!(
        clicks,
        vec![loc_op(8, 5, "Take From", 7000)],
        "the next round clicks the loc again"
    );
    iso.probe("globalThis.__ready = true").unwrap();
    snap.tick = 8;
    post(&iso, &snap);
    tick(&iso, 8);
    tick(&iso, 9);
    assert_eq!(iso.probe("__ok").unwrap(), "done");
    assert_eq!(logs(&iso), vec!["reach: opening blocking 'Door' at (7,5)"]);
    iso.join();
}

/// Frozen `reachThroughDoors` (`Reach.ts:193-198`): "I can't reach that!"
/// with no door to clear is `'unreachable'`.
#[test]
fn loc_op_cant_reach_without_a_door_is_unreachable() {
    let iso = spawn(LOC_OP);
    let actions = ["Take From".to_string()];
    let locs = [barrel(&actions, 6, 5, 1)];
    let mut snap = base(stand());
    snap.locs = &locs;
    post(&iso, &snap);
    tick(&iso, 1);
    tick(&iso, 2);
    assert_eq!(iso.drain_interacts(), vec![loc_op(6, 5, "Take From", 7000)]);
    let lines = cant_reach(3);
    snap.tick = 3;
    snap.chat_lines = &lines;
    post(&iso, &snap);
    let (ops, _) = tick_until_settled(&iso, 3, 6, std::time::Duration::ZERO);
    assert_eq!(iso.probe("__ok").unwrap(), "unreachable");
    assert_eq!(loc_clicks(&ops), 0, "no second click: {ops:?}");
    assert_eq!(
        logs(&iso),
        vec![
            "reach: 'Barrel of flour' at (6,5): server can't reach it and no door in front \
             to open or walk through (unreachable)"
        ]
    );
    iso.join();
}

/// Frozen `sceneSettled` (`Reach.ts:76-82, 244`): standing at the stand
/// with the loc not posted yet, the reach waits for the scene instead of
/// walking, then clicks the loc once it is posted.
#[test]
fn loc_op_at_the_stand_waits_for_the_scene_then_clicks() {
    let iso = spawn(LOC_OP);
    let mut snap = base(stand());
    post(&iso, &snap);
    tick(&iso, 1);
    tick(&iso, 2);
    assert!(iso.drain_interacts().is_empty(), "no walk, no click");
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);
    let actions = ["Take From".to_string()];
    let locs = [barrel(&actions, 6, 5, 1)];
    snap.tick = 3;
    snap.locs = &locs;
    post(&iso, &snap);
    let mut ops = Vec::new();
    for n in 3..=5 {
        tick(&iso, n);
        ops.extend(iso.drain_interacts());
    }
    assert_eq!(ops, vec![loc_op(6, 5, "Take From", 7000)]);
    iso.join();
}

/// Frozen `find()` keeps only the given `id` (`Reach.ts:242`): the nearer
/// same-name loc with another id is not clicked.
#[test]
fn loc_op_clicks_only_the_loc_with_the_given_id() {
    let iso = spawn(LOC_OP);
    let actions = ["Take From".to_string()];
    let mut other = barrel(&actions, 6, 5, 1);
    other.id = 7001;
    let locs = [other, barrel(&actions, 5, 8, 3)];
    let mut snap = base(stand());
    snap.locs = &locs;
    post(&iso, &snap);
    tick(&iso, 1);
    tick(&iso, 2);
    assert_eq!(iso.drain_interacts(), vec![loc_op(5, 8, "Take From", 7000)]);
    iso.join();
}

/// Frozen `walkResilient` awaits `Sustain.run()` on each ladder pass
/// (`Traversal.ts:130`): the stand walk pumps the script's sustain hook.
#[test]
fn loc_op_pumps_sustain_while_it_walks_to_the_stand() {
    let iso = spawn(LOC_OP);
    iso.probe("globalThis.__near = { x: 9, z: 5, level: 0 }")
        .unwrap();
    let actions = ["Take From".to_string()];
    let locs = [barrel(&actions, 10, 5, 5)];
    let mut snap = base(stand());
    snap.locs = &locs;
    post(&iso, &snap);
    tick(&iso, 1);
    match &iso.drain_interacts()[..] {
        [InteractReq::WalkNear {
            x: 9,
            z: 5,
            radius: 1,
            ..
        }] => {}
        other => panic!("expected the stand walk-near at radius 1, got {other:?}"),
    }
    for n in 2..=4 {
        snap.tick = n;
        post(&iso, &snap);
        tick(&iso, n);
    }
    let fed = iso.probe("__fed").unwrap().as_u64().unwrap();
    assert!(fed >= 3, "one sustain per walking tick, got {fed}");
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null, "still walking");
    iso.join();
}

/// One locOp with `expectMs` set to the JS literal `expect_ms`: asserts the
/// first click and an unsettled reach, and returns the clicks in the four
/// ticks after a 60 ms wait.
fn loc_op_clicks_after_a_wait(expect_ms: &str) -> usize {
    let iso = spawn(LOC_OP);
    iso.probe(&format!("globalThis.__expectMs = {expect_ms}"))
        .unwrap();
    let actions = ["Take From".to_string()];
    let locs = [barrel(&actions, 6, 5, 1)];
    let mut snap = base(stand());
    snap.locs = &locs;
    post(&iso, &snap);
    tick(&iso, 1);
    tick(&iso, 2);
    let first = iso.drain_interacts();
    std::thread::sleep(std::time::Duration::from_millis(60));
    let mut later = Vec::new();
    for n in 3..=6 {
        tick(&iso, n);
        later.extend(iso.drain_interacts());
    }
    let ok = iso.probe("__ok").unwrap();
    iso.join();
    assert_eq!(first, vec![loc_op(6, 5, "Take From", 7000)], "{expect_ms}");
    assert_eq!(ok, Value::Null, "{expect_ms}: still reaching, not {ok}");
    loc_clicks(&later)
}

/// Frozen `delayUntil(cond, expectMs)` arms a bound for any `expectMs > 0`
/// (`Execution.ts:19-21`): a fractional window is a window, not "not impl"
/// (review MIN-1). It times out and the next round clicks again.
#[test]
fn loc_op_fractional_expect_ms_times_out_then_clicks_again() {
    assert_eq!(loc_op_clicks_after_a_wait("0.5"), 1);
}

/// Frozen `delayUntil(cond, expectMs)` arms no bound for `expectMs <= 0`
/// or a non-number (`Execution.ts:19-21`), and an infinite bound never
/// ends: none of them times out at once or is "not impl" (review MIN-1).
#[test]
fn loc_op_zero_negative_or_non_finite_expect_ms_waits_without_a_bound() {
    for no_bound in ["0", "-1", "-0.5", "NaN", "Infinity"] {
        assert_eq!(loc_op_clicks_after_a_wait(no_bound), 0, "{no_bound}");
    }
}
