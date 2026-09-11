use script::isolate_fb::{
    encode_snapshot_delta, encode_snapshot_with_native, IsolateBuf, NativeFactsInput,
    QuestStatusInput, ReachViewInput, SnapshotInput, TileInput,
};
use script::{LoadIsolate, LoadShape};

fn base_snapshot<'a>() -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: Some(TileInput {
            x: 2575,
            z: 9893,
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
        side_tab: 2,
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
import { Quests } from '../../api/ui/questlog/Quests.js';
export default class T extends LoopingBot {
    loop() {
        let unavailable = false;
        let all = null;
        try { all = Quests.all(); } catch (_) { unavailable = true; }
        globalThis.__probe = {
            unavailable,
            all,
            waterfall: unavailable ? null : Quests.status('waterfall quest'),
            lostCity: unavailable ? null : Quests.status('LOST CITY'),
            missing: unavailable ? null : Quests.status('Missing Quest'),
        };
    }
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
fn quests_preserve_rows_and_frozen_status_strings_with_first_duplicate_winning() {
    let iso = spawn();
    let rows = [
        QuestStatusInput {
            name: "Quest Journal",
            status: "unknown",
        },
        QuestStatusInput {
            name: "Waterfall Quest",
            status: "notStarted",
        },
        QuestStatusInput {
            name: "Waterfall Quest",
            status: "complete",
        },
        QuestStatusInput {
            name: "Lost City",
            status: "inProgress",
        },
    ];
    let value = tick(
        &iso,
        encode_snapshot_with_native(
            &base_snapshot(),
            NativeFactsInput {
                quest_statuses: Some(&rows),
                ..Default::default()
            },
        ),
        1,
    );

    assert!(!value["unavailable"].as_bool().unwrap());
    assert_eq!(
        value["all"],
        serde_json::json!([
            { "name": "Quest Journal", "status": "unknown" },
            { "name": "Waterfall Quest", "status": "notStarted" },
            { "name": "Waterfall Quest", "status": "complete" },
            { "name": "Lost City", "status": "inProgress" }
        ])
    );
    assert_eq!(value["waterfall"], "notStarted");
    assert_eq!(value["lostCity"], "inProgress");
    assert_eq!(value["missing"], "unknown");
    iso.join();
}

#[test]
fn changed_snapshot_clears_a_missing_quest_tab_to_unavailable() {
    let iso = spawn();
    let rows = [QuestStatusInput {
        name: "Waterfall Quest",
        status: "complete",
    }];
    let mut snap = base_snapshot();
    let mut buffer = IsolateBuf::new();
    let (keyframe, fingerprint) = buffer.encode_snapshot_delta_with_native(
        None,
        &snap,
        NativeFactsInput {
            quest_statuses: Some(&rows),
            ..Default::default()
        },
        false,
    );
    assert_eq!(tick(&iso, keyframe, 1)["waterfall"], "complete");

    snap.tick = 2;
    let (unchanged, fingerprint) = buffer.encode_snapshot_delta_with_native(
        Some(&fingerprint),
        &snap,
        NativeFactsInput {
            quest_statuses: Some(&rows),
            ..Default::default()
        },
        false,
    );
    let unchanged_wire = script::isolate_fb::decode_snapshot(&unchanged).unwrap();
    assert!(
        !unchanged_wire.has_quest_statuses(),
        "unchanged rows must stay out of the delta"
    );
    assert_eq!(tick(&iso, unchanged, 2)["waterfall"], "complete");

    snap.tick = 3;
    let (delta, _) = buffer.encode_snapshot_delta_with_native(
        Some(&fingerprint),
        &snap,
        NativeFactsInput::default(),
        false,
    );
    let value = tick(&iso, delta, 3);
    assert_eq!(value["unavailable"], true);
    assert!(value["all"].is_null());
    iso.join();
}

#[test]
fn buffer_without_quest_rows_keeps_the_query_explicitly_unavailable() {
    let iso = spawn();
    let snap = base_snapshot();
    let (_, fingerprint) = encode_snapshot_delta(None, &snap, false);
    let (old_like_delta, _) = encode_snapshot_delta(Some(&fingerprint), &snap, false);

    let value = tick(&iso, old_like_delta, 1);
    assert_eq!(value["unavailable"], true);
    assert!(value["all"].is_null());
    iso.join();
}

#[test]
fn loaded_empty_quest_tab_remains_available() {
    let iso = spawn();
    let value = tick(
        &iso,
        encode_snapshot_with_native(
            &base_snapshot(),
            NativeFactsInput {
                quest_statuses: Some(&[]),
                ..Default::default()
            },
        ),
        1,
    );

    assert_eq!(value["unavailable"], false);
    assert_eq!(value["all"], serde_json::json!([]));
    assert_eq!(value["missing"], "unknown");
    iso.join();
}

#[test]
fn separate_isolates_do_not_share_quest_rows() {
    let first = spawn();
    let second = spawn();
    let rows = [QuestStatusInput {
        name: "Waterfall Quest",
        status: "complete",
    }];

    let first_value = tick(
        &first,
        encode_snapshot_with_native(
            &base_snapshot(),
            NativeFactsInput {
                quest_statuses: Some(&rows),
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

    assert_eq!(first_value["waterfall"], "complete");
    assert_eq!(second_value["unavailable"], true);
    assert!(second_value["all"].is_null());
    first.join();
    second.join();
}
