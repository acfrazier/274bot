use script::isolate_fb::{
    encode_snapshot, ReachViewInput, SceneEntityInput, SnapshotInput, TileInput,
};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};

fn snapshot<'a>(ground: &'a [SceneEntityInput<'a>]) -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: Some(TileInput {
            x: 3200,
            z: 3200,
            level: 0,
        }),
        ingame: true,
        inv: &[],
        inv_size: 28,
        stats: &[],
        booths: &[],
        nearest_booth: None,
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
        ground,
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
        my_name: Some("alice"),
        in_combat: false,
        animating: false,
        main_modal_id: -1,
        chat_modal_id: -1,
        make_products: &[],
        side_tab_ifaces: &[],
        spell_buttons: &[],
        chat_lines: &[],
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

fn row(id: i32, level: i32, actions: &[String]) -> SceneEntityInput<'_> {
    SceneEntityInput {
        index: id,
        id,
        name: Some("Halloween mask"),
        x: 3201,
        z: 3200,
        level,
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

fn run(expression: &str, ground: &[SceneEntityInput<'_>]) -> (serde_json::Value, Vec<InteractReq>) {
    let source = format!(
        "import {{ Input }} from '../../input/Input.js';\n\
         export default class T extends LoopingBot {{\n\
           loop() {{ globalThis.__probe = {expression}; }}\n\
         }}"
    );
    let iso = LoadIsolate::spawn(source, LoadShape::CompatClass, vec![]).unwrap();
    iso.post_snapshot(encode_snapshot(&snapshot(ground)));
    iso.on_game_tick(1);
    let probe = iso.probe("__probe").unwrap();
    let requests = iso.drain_interacts();
    iso.join();
    (probe, requests)
}

#[test]
fn input_take_obj_selects_each_exact_mask_and_coerces_id_and_slot() {
    let actions = ["".into(), "Hidden".into(), "Take".into()];
    let ground = [
        row(1053, 0, &actions),
        row(1055, 0, &actions),
        row(1057, 0, &actions),
    ];
    for id in [1053, 1055, 1057] {
        let (probe, requests) = run(&format!("Input.takeObj(3201, 3200, '{id}', '3')"), &ground);
        assert_eq!(probe, true);
        assert_eq!(
            requests,
            [InteractReq::Obj {
                x: 3201,
                z: 3200,
                level: 0,
                name: Some("Halloween mask".into()),
                action: "Take".into(),
                id: Some(id),
            }]
        );
    }
}

#[test]
fn input_take_obj_rejects_missing_id_wrong_tile_or_wrong_plane() {
    let actions = ["Take".into()];
    let ground = [row(1053, 0, &actions), row(1055, 1, &actions)];
    for expression in [
        "Input.takeObj(3201, 3200, 1057, 1)",
        "Input.takeObj(3201, 3200, 1055, 1)",
        "Input.takeObj(3202, 3200, 1053, 1)",
        "Input.takeObj(3201, 3201, 1053, 1)",
        "Input.takeObj(3201, 3200, NaN, 1)",
        "Input.takeObj(3201, 3200, 1053.5, 1)",
    ] {
        let (probe, requests) = run(expression, &ground);
        assert_eq!(probe, false, "{expression}");
        assert!(requests.is_empty(), "{expression}");
    }
}

#[test]
fn input_take_obj_preserves_action_gaps_and_boolean_refusals() {
    let actions = [
        "".into(),
        "hIdDeN".into(),
        "Take".into(),
        "".into(),
        "Examine".into(),
    ];
    let ground = [row(1055, 0, &actions)];
    for op in [
        "0",
        "-1",
        "1",
        "2",
        "4",
        "6",
        "1.5",
        "NaN",
        "Infinity",
        "'invalid'",
    ] {
        let (probe, requests) = run(&format!("Input.takeObj(3201, 3200, 1055, {op})"), &ground);
        assert_eq!(probe, false, "op {op}");
        assert!(requests.is_empty(), "op {op}");
    }
    let (probe, requests) = run("Input.takeObj(3201, 3200, 1055, 5)", &ground);
    assert_eq!(probe, true);
    assert!(
        matches!(&requests[..], [InteractReq::Obj { id: Some(1055), action, .. }] if action == "Examine")
    );
}
