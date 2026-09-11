use script::isolate_fb::{
    encode_snapshot, encode_snapshot_delta, encode_snapshot_with_native, IsolateBuf,
    NativeFactsInput, ReachViewInput, SnapshotInput, TileInput,
};
use script::shim::InteractReq;
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

fn spawn(body: &str) -> LoadIsolate {
    let src = format!(
        r#"
import {{ reader, actions }} from '../../adapter/ClientAdapter.js';
export default class T extends LoopingBot {{
    loop() {{ {body} }}
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

#[test]
fn reader_returns_posted_local_overhead_and_coordinate_hint_unchanged() {
    let iso =
        spawn("globalThis.__probe = { selfChat: reader.selfChat(), hintTile: reader.hintTile() };");
    let mut snap = base_snapshot();
    snap.chat_text = Some("latest ring line");
    let facts = NativeFactsInput {
        self_chat: Some("FIGHT!"),
        hint_tile: Some((2761, 9546)),
    };

    let value = tick(&iso, encode_snapshot_with_native(&snap, facts), 1);
    assert_eq!(value["selfChat"], "FIGHT!");
    assert_eq!(
        value["hintTile"],
        serde_json::json!({ "x": 2761, "z": 9546 })
    );
    iso.join();
}

#[test]
fn successive_snapshot_clears_overhead_and_hint_without_stale_retention() {
    let iso =
        spawn("globalThis.__probe = { selfChat: reader.selfChat(), hintTile: reader.hintTile() };");
    let mut snap = base_snapshot();
    let first_facts = NativeFactsInput {
        self_chat: Some("3"),
        hint_tile: Some((2800, 9500)),
    };
    let mut buffer = IsolateBuf::new();
    let (keyframe, fingerprint) =
        buffer.encode_snapshot_delta_with_native(None, &snap, first_facts, false);
    let first = tick(&iso, keyframe, 1);
    assert_eq!(first["selfChat"], "3");
    assert_eq!(
        first["hintTile"],
        serde_json::json!({ "x": 2800, "z": 9500 })
    );

    snap.tick = 2;
    let (delta, _) = buffer.encode_snapshot_delta_with_native(
        Some(&fingerprint),
        &snap,
        NativeFactsInput::default(),
        false,
    );
    let second = tick(&iso, delta, 2);
    assert!(second["selfChat"].is_null());
    assert!(second["hintTile"].is_null());
    iso.join();
}

#[test]
fn absent_old_buffer_defaults_new_reader_facts_to_null() {
    let iso =
        spawn("globalThis.__probe = { selfChat: reader.selfChat(), hintTile: reader.hintTile() };");
    let snap = base_snapshot();
    let (_, fingerprint) = encode_snapshot_delta(None, &snap, false);
    let (old_like_delta, _) = encode_snapshot_delta(Some(&fingerprint), &snap, false);

    let value = tick(&iso, old_like_delta, 1);
    assert!(value["selfChat"].is_null());
    assert!(value["hintTile"].is_null());
    iso.join();
}

#[test]
fn set_run_queues_existing_command_in_call_order() {
    let iso = spawn("globalThis.__probe = [actions.setRun(true), actions.setRun(0)];");
    let value = tick(&iso, encode_snapshot(&base_snapshot()), 1);
    assert_eq!(value, serde_json::json!([true, true]));
    assert_eq!(
        iso.drain_interacts(),
        vec![
            InteractReq::SetRun { on: true },
            InteractReq::SetRun { on: false },
        ]
    );
    iso.join();
}
