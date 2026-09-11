use script::isolate_fb::{
    encode_snapshot_delta, encode_snapshot_with_native, IsolateBuf, NativeFactsInput,
    ReachViewInput, SnapshotInput, TileInput,
};
use script::{LoadIsolate, LoadShape};

fn base_snapshot<'a>() -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: Some(TileInput {
            x: 2805,
            z: 9590,
            level: 3,
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

fn spawn() -> LoadIsolate {
    let src = r#"
import { reader } from '../../adapter/ClientAdapter.js';
export default class T extends LoopingBot {
    loop() { globalThis.__probe = reader.retaliateControls(); }
}
"#;
    LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn tick(iso: &LoadIsolate, bytes: Vec<u8>, tick: u64) -> serde_json::Value {
    iso.post_snapshot(bytes);
    iso.on_game_tick(tick);
    let _ = iso.probe("true");
    iso.probe("__probe").unwrap()
}

#[test]
fn reader_returns_posted_retaliate_pair_without_guessing_from_enabled_state() {
    let iso = spawn();
    let facts = NativeFactsInput {
        retaliate_controls: Some((0x12345, 0x23456)),
        ..Default::default()
    };

    let value = tick(
        &iso,
        encode_snapshot_with_native(&base_snapshot(), facts),
        1,
    );
    assert_eq!(
        value,
        serde_json::json!({ "onComId": 0x12345, "offComId": 0x23456 })
    );
    iso.join();
}

#[test]
fn changed_snapshot_clears_unavailable_retaliate_pair() {
    let iso = spawn();
    let mut snap = base_snapshot();
    let mut buffer = IsolateBuf::new();
    let (keyframe, fingerprint) = buffer.encode_snapshot_delta_with_native(
        None,
        &snap,
        NativeFactsInput {
            retaliate_controls: Some((123, 456)),
            ..Default::default()
        },
        false,
    );
    assert_eq!(
        tick(&iso, keyframe, 1),
        serde_json::json!({ "onComId": 123, "offComId": 456 })
    );

    snap.tick = 2;
    let (delta, _) = buffer.encode_snapshot_delta_with_native(
        Some(&fingerprint),
        &snap,
        NativeFactsInput::default(),
        false,
    );
    assert!(tick(&iso, delta, 2).is_null());
    iso.join();
}

#[test]
fn buffer_without_retaliate_fields_defaults_reader_to_null() {
    let iso = spawn();
    let snap = base_snapshot();
    let (_, fingerprint) = encode_snapshot_delta(None, &snap, false);
    let (old_like_delta, _) = encode_snapshot_delta(Some(&fingerprint), &snap, false);

    assert!(tick(&iso, old_like_delta, 1).is_null());
    iso.join();
}

#[test]
fn separate_isolates_do_not_share_retaliate_control_identity() {
    let first = spawn();
    let second = spawn();
    let first_value = tick(
        &first,
        encode_snapshot_with_native(
            &base_snapshot(),
            NativeFactsInput {
                retaliate_controls: Some((301, 302)),
                ..Default::default()
            },
        ),
        1,
    );
    let second_value = tick(
        &second,
        encode_snapshot_with_native(&base_snapshot(), NativeFactsInput::default()),
        1,
    );

    assert_eq!(
        first_value,
        serde_json::json!({ "onComId": 301, "offComId": 302 })
    );
    assert!(second_value.is_null());
    first.join();
    second.join();
}
