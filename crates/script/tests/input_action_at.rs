// Numeric Input.interact* ops use native 1-based slot indices; sparse holes stay empty.

use script::isolate_fb::{
    ReachViewInput, SceneEntityInput, SnapshotInput, TileInput,
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
            x: 3222,
            z: 3222,
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

fn player_row<'a>(
    index: i32,
    name: &'a str,
    actions: &'a [String],
) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index,
        id: index,
        name: Some(name),
        x: 3222,
        z: 3222,
        level: 0,
        distance: 1,
        health: 99,
        max_health: 99,
        in_combat: false,
        animating: false,
        actions,
        reachable: true,
        reachable_adj: true,
        combat_level: 42,
        target_kind: 0,
        target_index: -1,
        size: 0,
        nx: 0,
        nz: 0,
    }
}

fn duel_pen_actions() -> [String; 5] {
    [
        String::new(),
        "Fight".into(),
        "Follow".into(),
        "Trade with".into(),
        String::new(),
    ]
}

#[test]
fn input_interact_player_sparse_slot_fight_is_op_two_not_follow() {
    let src = r#"
import { Input } from '../../input/Input.js';
export default class T extends LoopingBot {
    loop() { globalThis.__probe = Input.interactPlayer(2, 2); }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let actions = duel_pen_actions();
    let players = [player_row(2, "rival", &actions)];
    let mut snap = base_snapshot();
    snap.players = &players;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__probe").unwrap(), true);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Player {
            name: "rival".into(),
            action: "Fight".into(),
        }],
        "native slot 2 is Fight; presentOps compaction would have queued Follow"
    );
    iso.join();
}

#[test]
fn input_interact_player_rejects_empty_and_hidden_slots() {
    let src = r#"
import { Input } from '../../input/Input.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__empty = Input.interactPlayer(2, 1);
        globalThis.__hidden = Input.interactPlayer(2, 4);
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let actions = [
        String::new(),
        "Fight".into(),
        "Follow".into(),
        "hidden".into(),
        String::new(),
    ];
    let players = [player_row(2, "rival", &actions)];
    let mut snap = base_snapshot();
    snap.players = &players;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__empty").unwrap(), false);
    assert_eq!(iso.probe("__hidden").unwrap(), false);
    assert!(
        iso.drain_interacts().is_empty(),
        "absent and hidden native slots must not queue"
    );
    iso.join();
}

#[test]
fn input_interact_player_challenge_still_op_one() {
    let src = r#"
import { Input } from '../../input/Input.js';
export default class T extends LoopingBot {
    loop() { globalThis.__probe = Input.interactPlayer(2, 1); }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let actions = [
        "Challenge".into(),
        "Follow".into(),
        "Trade with".into(),
    ];
    let players = [player_row(2, "rival", &actions)];
    let mut snap = base_snapshot();
    snap.players = &players;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__probe").unwrap(), true);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Player {
            name: "rival".into(),
            action: "Challenge".into(),
        }]
    );
    iso.join();
}

#[test]
fn input_interact_npc_sparse_slot_uses_native_index() {
    let src = r#"
import { Input } from '../../input/Input.js';
export default class T extends LoopingBot {
    loop() { globalThis.__probe = Input.interactNpc(5, 2); }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let actions = [
        String::new(),
        "Talk-to".into(),
        "Attack".into(),
    ];
    let npcs = [SceneEntityInput {
        index: 5,
        id: 1,
        name: Some("Guard"),
        x: 3222,
        z: 3222,
        level: 0,
        distance: 1,
        health: 10,
        max_health: 10,
        in_combat: false,
        animating: false,
        actions: &actions,
        reachable: true,
        reachable_adj: true,
        combat_level: 21,
        target_kind: 0,
        target_index: -1,
        size: 0,
        nx: 0,
        nz: 0,
    }];
    let mut snap = base_snapshot();
    snap.npcs = &npcs;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__probe").unwrap(), true);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Npc {
            name: "Guard".into(),
            action: "Talk-to".into(),
            index: Some(5),
        }]
    );
    iso.join();
}

#[test]
fn input_interact_loc_sparse_slot_uses_native_index() {
    let src = r#"
import { Input } from '../../input/Input.js';
export default class T extends LoopingBot {
    loop() { globalThis.__probe = Input.interactLoc(3200, 3200, 0, 3); }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let actions = [
        String::new(),
        String::new(),
        "Open".into(),
    ];
    let locs = [SceneEntityInput {
        index: 0,
        id: 100,
        name: Some("Door"),
        x: 3200,
        z: 3200,
        level: 0,
        distance: 1,
        health: 0,
        max_health: 0,
        in_combat: false,
        animating: false,
        actions: &actions,
        reachable: true,
        reachable_adj: true,
        combat_level: 0,
        target_kind: 0,
        target_index: -1,
        size: 0,
        nx: 0,
        nz: 0,
    }];
    let mut snap = base_snapshot();
    snap.locs = &locs;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__probe").unwrap(), true);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Loc {
            x: 3200,
            z: 3200,
            level: 0,
            action: "Open".into(),
            id: None,
        }]
    );
    iso.join();
}
