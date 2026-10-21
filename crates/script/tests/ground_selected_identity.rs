use script::isolate_fb::{
    decode_interact_batch, encode_interact_batch, ReachViewInput, SceneEntityInput, SnapshotInput,
    TileInput,
};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};

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

fn ground_row<'a>(id: i32, name: &'a str, actions: &'a [String]) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index: id,
        id,
        name: Some(name),
        x: 2737,
        z: 3440,
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
        size: 0,
        nx: 0,
        nz: 0,
        shape: 0,
        angle: 0,
    }
}

fn posted_ground_interactions() -> Vec<InteractReq> {
    let src = r#"
import { GroundItems } from '../../api/grounditems/GroundItems.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = GroundItems.query().results().map(item => item.interact('Take'));
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let take = ["Take".to_string()];
    let ground = [
        ground_row(1053, "Coins", &take),
        ground_row(1055, "Coins", &take),
        ground_row(1057, "Coins", &take),
    ];
    let mut snap = base_snapshot();
    snap.ground = &ground;
    iso.post_snapshot(script::isolate_fb::encode_snapshot(&snap));
    iso.on_game_tick(1);
    assert_eq!(
        iso.probe("__probe").unwrap(),
        serde_json::json!([true, true, true])
    );
    let reqs = iso.drain_interacts();
    iso.join();
    reqs
}

fn assert_cold_ground_request_has_fast_properties() {
    let src = r#"
export default class T extends LoopingBot {
    loop() {
        const row = {
            id: 1053, x: 2737, z: 3440, level: 0,
            name: 'Coins', actions: ['Take'],
        };
        globalThis.__probe = __rs2b0t_entities_v8(
            57, { snap: row }, 'Take',
            request => { globalThis.__cold_ground_request = request; },
        );
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__probe").unwrap(), true);
    assert_eq!(
        iso.probe("new Function('o', 'return %HasFastProperties(o)')(__cold_ground_request)")
            .unwrap(),
        true,
        "native cold request creation must create fast properties without JS warming"
    );
    iso.join();
}
fn assert_cold_input_take_obj_request_has_fast_properties_on_posted_page() {
    let src = r#"
import { Input } from '../../input/Input.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = Input.takeObj(2737, 3440, 1053, 1);
        globalThis.__cold_input_request = __rs2b0t_host.interact[0];
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let take = ["Take".to_string()];
    let ground = [ground_row(1053, "Coins", &take)];
    let mut snap = base_snapshot();
    snap.ground = &ground;
    iso.post_snapshot(script::isolate_fb::encode_snapshot(&snap));
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__probe").unwrap(), true);
    assert_eq!(
        iso.probe("new Function('o', 'return %HasFastProperties(o)')(__cold_input_request)")
            .unwrap(),
        true,
        "cold Input.takeObj binding request must have fast properties"
    );
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Obj {
            x: 2737,
            z: 3440,
            level: 0,
            name: Some("Coins".into()),
            action: "Take".into(),
            id: Some(1053),
        }],
        "Input.takeObj must select the posted ground row"
    );
    iso.join();
}

fn ground_item_interact_emits_the_selected_posted_id_through_flatbuffer() {
    assert_cold_ground_request_has_fast_properties();
    let reqs = posted_ground_interactions();
    assert_eq!(
        reqs,
        vec![
            InteractReq::Obj {
                x: 2737,
                z: 3440,
                level: 0,
                name: Some("Coins".into()),
                action: "Take".into(),
                id: Some(1053),
            },
            InteractReq::Obj {
                x: 2737,
                z: 3440,
                level: 0,
                name: Some("Coins".into()),
                action: "Take".into(),
                id: Some(1055),
            },
            InteractReq::Obj {
                x: 2737,
                z: 3440,
                level: 0,
                name: Some("Coins".into()),
                action: "Take".into(),
                id: Some(1057),
            },
        ],
        "same-name co-located wrappers must retain their distinct posted row identities"
    );
    let bytes = encode_interact_batch(&reqs);
    let decoded = decode_interact_batch(&bytes).expect("ground interaction batch decodes");
    assert_eq!(decoded, reqs);
    assert_eq!(
        decoded
            .iter()
            .map(|request| match request {
                InteractReq::Obj { id, .. } => *id,
                _ => panic!("expected object interaction"),
            })
            .collect::<Vec<_>>(),
        vec![Some(1053), Some(1055), Some(1057)]
    );
}

fn ground_item_interact_with_missing_action_returns_false_without_queueing() {
    let src = r#"
import { GroundItems } from '../../api/grounditems/GroundItems.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = GroundItems.query().first().interact('Take');
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let actions = ["Examine".to_string()];
    let ground = [ground_row(1053, "Coins", &actions)];
    let mut snap = base_snapshot();
    snap.ground = &ground;
    iso.post_snapshot(script::isolate_fb::encode_snapshot(&snap));
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__probe").unwrap(), false);
    assert!(
        iso.drain_interacts().is_empty(),
        "unavailable action queues nothing"
    );
    iso.join();
}

fn object_id_round_trips_as_some_and_none_and_defaults_when_omitted() {
    let requests = [
        InteractReq::Obj {
            x: 2737,
            z: 3440,
            level: 0,
            name: Some("Coins".into()),
            action: "Take".into(),
            id: Some(1053),
        },
        InteractReq::Obj {
            x: 2737,
            z: 3440,
            level: 0,
            name: Some("Coins".into()),
            action: "Take".into(),
            id: None,
        },
    ];
    let bytes = encode_interact_batch(&requests);
    assert_eq!(
        decode_interact_batch(&bytes).expect("object batch decodes"),
        requests
    );
    let legacy: InteractReq = serde_json::from_value(serde_json::json!({
        "op": "obj",
        "x": 2737,
        "z": 3440,
        "level": 0,
        "name": "Coins",
        "action": "Take"
    }))
    .expect("legacy object request without id deserializes");
    assert_eq!(
        legacy,
        InteractReq::Obj {
            x: 2737,
            z: 3440,
            level: 0,
            name: Some("Coins".into()),
            action: "Take".into(),
            id: None,
        }
    );
}
fn main() {
    // V8 must enable native syntax before process-main bootstrap for `%HasFastProperties`.
    v8::V8::set_flags_from_string("--allow-natives-syntax");
    script::bootstrap_v8_platform().expect("process-main V8 bootstrap");

    assert_cold_input_take_obj_request_has_fast_properties_on_posted_page();
    println!("PASS cold Input.takeObj binding fast properties on posted ground page");
    ground_item_interact_emits_the_selected_posted_id_through_flatbuffer();
    println!("PASS ground identity, FlatBuffer batch, and cold fast properties");
    ground_item_interact_with_missing_action_returns_false_without_queueing();
    println!("PASS unavailable ground action");
    object_id_round_trips_as_some_and_none_and_defaults_when_omitted();
    println!("PASS object id Some/None wire and omitted serde compatibility");
}
