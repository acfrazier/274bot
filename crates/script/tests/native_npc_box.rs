use script::isolate_fb::{
    decode_snapshot, encode_snapshot_delta_with_native, encode_snapshot_with_native,
    NativeFactsInput, NpcBoxInput, ReachViewInput, SnapshotInput,
};
use script::{LoadIsolate, LoadShape};

fn base_snapshot<'a>() -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: None,
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

fn spawn(body: &str) -> LoadIsolate {
    let src = format!(
        r#"
import {{ reader }} from '../../adapter/ClientAdapter.js';
export default class T extends LoopingBot {{
    loop() {{}}
    onPaint() {{ {body} }}
}}
"#
    );
    LoadIsolate::spawn(src, LoadShape::CompatClass, vec![]).unwrap()
}

fn tick(iso: &LoadIsolate, bytes: Vec<u8>, tick: u64) -> serde_json::Value {
    iso.post_snapshot(bytes);
    iso.on_game_tick(tick);
    let _ = iso.probe("true");
    iso.probe("__probe").unwrap()
}

const POINTS: [(i32, i32); 8] = [
    (225, 171),
    (295, 171),
    (290, 171),
    (230, 171),
    (225, 117),
    (295, 117),
    (290, 123),
    (230, 123),
];

#[test]
fn reader_returns_posted_npc_box_as_fresh_points() {
    let iso = spawn(
        "const first = reader.npcBox(7); first[0].x = -1; globalThis.__probe = [first, reader.npcBox(7), reader.npcBox(8)];",
    );
    let rows = [NpcBoxInput {
        index: 7,
        points: POINTS,
    }];
    let bytes = encode_snapshot_with_native(
        &base_snapshot(),
        NativeFactsInput {
            npc_boxes: Some(&rows),
            ..Default::default()
        },
    );

    let value = tick(&iso, bytes, 1);
    assert_eq!(value[0][0]["x"], -1);
    assert_eq!(
        value[1],
        serde_json::json!([
            {"x":225,"y":171}, {"x":295,"y":171},
            {"x":290,"y":171}, {"x":230,"y":171},
            {"x":225,"y":117}, {"x":295,"y":117},
            {"x":290,"y":123}, {"x":230,"y":123}
        ])
    );
    assert!(value[2].is_null());
    iso.join();
}

#[test]
fn unavailable_update_clears_a_previously_posted_box() {
    let iso = spawn("globalThis.__probe = reader.npcBox(7);");
    let rows = [NpcBoxInput {
        index: 7,
        points: POINTS,
    }];
    let first = encode_snapshot_with_native(
        &base_snapshot(),
        NativeFactsInput {
            npc_boxes: Some(&rows),
            ..Default::default()
        },
    );
    assert!(tick(&iso, first, 1).is_array());

    let mut next = base_snapshot();
    next.tick = 2;
    let cleared = encode_snapshot_with_native(&next, NativeFactsInput::default());
    assert!(tick(&iso, cleared, 2).is_null());
    iso.join();
}

#[test]
fn old_buffer_without_npc_box_fields_initializes_null() {
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let table = builder.start_table();
    builder.push_slot_always(4, 1_u64);
    let root = builder.end_table(table);
    builder.finish(root, None);

    let iso = spawn("globalThis.__probe = reader.npcBox(7);");
    assert!(tick(&iso, builder.finished_data().to_vec(), 1).is_null());
    iso.join();
}

#[test]
fn npc_boxes_are_isolated_per_script_slot() {
    let rows = [NpcBoxInput {
        index: 7,
        points: POINTS,
    }];
    let with_box = encode_snapshot_with_native(
        &base_snapshot(),
        NativeFactsInput {
            npc_boxes: Some(&rows),
            ..Default::default()
        },
    );
    let without_box = encode_snapshot_with_native(&base_snapshot(), NativeFactsInput::default());
    let first = spawn("globalThis.__probe = reader.npcBox(7);");
    let second = spawn("globalThis.__probe = reader.npcBox(7);");

    assert!(tick(&first, with_box, 1).is_array());
    assert!(tick(&second, without_box, 1).is_null());
    first.join();
    second.join();
}

#[test]
fn unchanged_delta_retains_and_despawn_delta_removes_the_box() {
    let rows = [NpcBoxInput {
        index: 7,
        points: POINTS,
    }];
    let native = NativeFactsInput {
        npc_boxes: Some(&rows),
        ..Default::default()
    };
    let (first, fingerprint) =
        encode_snapshot_delta_with_native(None, &base_snapshot(), native, false);
    let iso = spawn("globalThis.__probe = reader.npcBox(7);");
    assert!(tick(&iso, first, 1).is_array());

    let mut next = base_snapshot();
    next.tick = 2;
    let (unchanged, fingerprint) =
        encode_snapshot_delta_with_native(Some(&fingerprint), &next, native, false);
    assert!(!decode_snapshot(&unchanged).unwrap().has_npc_boxes_update());
    assert!(tick(&iso, unchanged, 2).is_array());

    next.tick = 3;
    let empty: [NpcBoxInput; 0] = [];
    let (despawned, _) = encode_snapshot_delta_with_native(
        Some(&fingerprint),
        &next,
        NativeFactsInput {
            npc_boxes: Some(&empty),
            ..Default::default()
        },
        false,
    );
    let update = decode_snapshot(&despawned).unwrap();
    assert!(update.has_npc_boxes_update());
    assert!(update.npc_boxes_available());
    assert!(update.npc_boxes().is_empty());
    assert!(tick(&iso, despawned, 3).is_null());
    iso.join();
}
