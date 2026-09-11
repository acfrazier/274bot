use script::isolate_fb::{
    decode_interact_batch, encode_interact_batch, ReachViewInput, SceneEntityInput, SnapshotInput,
    TileInput,
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
            x: 2738,
            z: 3441,
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

fn loc_row<'a>(
    id: i32,
    name: Option<&'a str>,
    x: i32,
    z: i32,
    actions: &'a [String],
) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index: id,
        id,
        name,
        x,
        z,
        level: 0,
        distance: 1,
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

const PICK: &str = r#"
import { Locs } from '../../api/locs/Locs.js';
export default class T extends LoopingBot {
    loop() {
        const flax = Locs.query().name('Flax').action('Pick').nearest();
        globalThis.__probe = flax ? flax.interact('Pick') : false;
    }
}
"#;

fn flax_pick(wall_first: bool) -> Vec<InteractReq> {
    let src = PICK;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let none: [String; 0] = [];
    let pick = ["Pick".to_string()];
    let wall = loc_row(980, None, 2737, 3440, &none);
    let flax = loc_row(2646, Some("Flax"), 2737, 3440, &pick);
    let locs = if wall_first {
        [wall, flax]
    } else {
        [flax, wall]
    };
    let mut snap = base_snapshot();
    snap.locs = &locs;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let probe = iso.probe("__probe").unwrap();
    assert_eq!(probe, true, "selected flax Pick must queue");
    let reqs = iso.drain_interacts();
    iso.join();
    reqs
}

#[test]
fn loc_interact_queues_selected_id_with_wall_first() {
    assert_eq!(
        flax_pick(true),
        vec![InteractReq::Loc {
            x: 2737,
            z: 3440,
            level: 0,
            action: "Pick".into(),
            id: Some(2646),
        }],
        "wall-first posted rows still queue Flax 2646"
    );
}

#[test]
fn loc_interact_queues_selected_id_with_flax_first() {
    assert_eq!(
        flax_pick(false),
        vec![InteractReq::Loc {
            x: 2737,
            z: 3440,
            level: 0,
            action: "Pick".into(),
            id: Some(2646),
        }],
        "flax-first posted rows still queue Flax 2646"
    );
}

#[test]
fn loc_interact_missing_action_does_not_queue() {
    let src = r#"
import { Locs } from '../../api/locs/Locs.js';
export default class T extends LoopingBot {
    loop() {
        const flax = Locs.query().name('Flax').nearest();
        globalThis.__probe = flax ? flax.interact('Open') : false;
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let pick = ["Pick".to_string()];
    let locs = [loc_row(2646, Some("Flax"), 2737, 3440, &pick)];
    let mut snap = base_snapshot();
    snap.locs = &locs;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__probe").unwrap(), false);
    assert!(
        iso.drain_interacts().is_empty(),
        "absent action must not queue a loc op"
    );
    iso.join();
}

#[test]
fn loc_interact_round_trips_selected_id_through_flatbuffer() {
    let reqs = flax_pick(true);
    let bytes = encode_interact_batch(&reqs);
    let got = decode_interact_batch(&bytes).expect("loc batch decodes");
    assert_eq!(got, reqs);
    assert_eq!(
        got[0],
        InteractReq::Loc {
            x: 2737,
            z: 3440,
            level: 0,
            action: "Pick".into(),
            id: Some(2646),
        }
    );
}

#[test]
fn loc_interact_legacy_without_id_round_trips() {
    let reqs = vec![InteractReq::Loc {
        x: 2737,
        z: 3440,
        level: 0,
        action: "Pick".into(),
        id: None,
    }];
    let bytes = encode_interact_batch(&reqs);
    let got = decode_interact_batch(&bytes).expect("legacy loc batch decodes");
    assert_eq!(got, reqs);
}
