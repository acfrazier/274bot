use script::isolate_fb::{
    encode_snapshot, encode_snapshot_delta, encode_snapshot_with_native, IsolateBuf,
    MainModalTextsInput, NativeFactsInput, QuestStatusInput, ReachViewInput, SnapshotInput,
    TileInput,
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

fn spawn() -> LoadIsolate {
    let src = r#"
import { Quests } from '@rs2b0t/api';
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

/// Reads the raw isolate page (`__rs2b0t_host.snapshot`) so a test can see
/// an omitted property as omitted — the v2 helpers copy fields and cannot.
fn spawn_page_probe() -> LoadIsolate {
    let src = r#"
export default class T extends LoopingBot {
    loop() {
        const page = globalThis.__rs2b0t_host ? globalThis.__rs2b0t_host.snapshot : null;
        const hasPair = !!page
            && Object.prototype.hasOwnProperty.call(page, 'main_modal_texts');
        const rows = page && Array.isArray(page.quest_statuses) ? page.quest_statuses : null;
        globalThis.__probe = {
            hasPair,
            pair: hasPair ? page.main_modal_texts : null,
            ids: rows
                ? rows.map((row) => Object.prototype.hasOwnProperty.call(row, 'component_id')
                    ? row.component_id
                    : 'absent')
                : null,
            mainModalId: page ? page.main_modal_id : null,
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
            component_id: Some(1),
        },
        QuestStatusInput {
            name: "Waterfall Quest",
            status: "notStarted",
            component_id: Some(42),
        },
        QuestStatusInput {
            name: "Waterfall Quest",
            status: "complete",
            component_id: Some(43),
        },
        QuestStatusInput {
            name: "Lost City",
            status: "inProgress",
            component_id: Some(44),
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
    // v1 `Quests.all` still copies `{ name, status }` only: the posted
    // click target is not spread into the v1 row.
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
fn quest_rows_post_the_walked_id_and_omit_an_absent_slot() {
    let iso = spawn_page_probe();
    let rows = [
        QuestStatusInput {
            name: "Quest Journal",
            status: "unknown",
            component_id: Some(6),
        },
        QuestStatusInput {
            name: "Waterfall Quest",
            status: "notStarted",
            component_id: None,
        },
        QuestStatusInput {
            name: "Waterfall Quest",
            status: "complete",
            component_id: Some(0),
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

    // A present id is posted (`6`), an absent slot omits the property
    // (never `0` or `-1`), and a present `0` is a real id and is posted.
    assert_eq!(
        value["ids"],
        serde_json::json!([6, "absent", 0]),
        "{value:?}"
    );
    iso.join();
}

#[test]
fn main_modal_texts_posts_the_pair_and_a_delta_without_it_keeps_the_page_copied() {
    let iso = spawn_page_probe();
    let texts = vec!["@red@Journal line".to_string(), "  second".to_string()];
    let mut snap = base_snapshot();
    snap.main_modal_id = 1234;
    let mut buffer = IsolateBuf::new();
    let (keyframe, fingerprint) = buffer.encode_snapshot_delta_with_native(
        None,
        &snap,
        NativeFactsInput {
            main_modal_texts: Some(MainModalTextsInput {
                root: 1234,
                texts: &texts,
            }),
            ..Default::default()
        },
        false,
    );
    let value = tick(&iso, keyframe, 1);
    assert_eq!(value["hasPair"], true, "{value:?}");
    // Walk order, colour tags intact, nothing filtered.
    assert_eq!(
        value["pair"],
        serde_json::json!({ "root": 1234, "texts": ["@red@Journal line", "  second"] }),
        "{value:?}"
    );
    assert_eq!(value["mainModalId"], 1234, "{value:?}");

    // The next post does not supply the pair. That omits the slot: the page
    // keeps its last pair, and the omission is not read as a closed modal.
    snap.tick = 2;
    let (delta, _) = buffer.encode_snapshot_delta_with_native(
        Some(&fingerprint),
        &snap,
        NativeFactsInput::default(),
        false,
    );
    let wire = script::isolate_fb::decode_snapshot(&delta).unwrap();
    assert!(
        !wire.has_main_modal_texts(),
        "an unsupplied pair must omit the slot, not write the closed object"
    );
    let value = tick(&iso, delta, 2);
    assert_eq!(value["hasPair"], true, "{value:?}");
    assert_eq!(
        value["pair"],
        serde_json::json!({ "root": 1234, "texts": ["@red@Journal line", "  second"] }),
        "{value:?}"
    );
    iso.join();
}

#[test]
fn main_modal_texts_lifecycle_posts_a_text_change_a_closed_pair_and_the_table_root() {
    let iso = spawn_page_probe();
    let texts_a = vec!["@red@line a".to_string()];
    let texts_b = vec!["@red@line a".to_string(), "line b".to_string()];
    let mut snap = base_snapshot();
    snap.main_modal_id = 77;
    let mut buffer = IsolateBuf::new();
    let (keyframe, fingerprint) = buffer.encode_snapshot_delta_with_native(
        None,
        &snap,
        NativeFactsInput {
            main_modal_texts: Some(MainModalTextsInput {
                root: 77,
                texts: &texts_a,
            }),
            ..Default::default()
        },
        false,
    );
    assert_eq!(
        tick(&iso, keyframe, 1)["pair"]["texts"],
        serde_json::json!(["@red@line a"])
    );

    // A text-only change still co-posts slot 68 with the table's root: a
    // delta can never leave lines under another root's id.
    snap.tick = 2;
    let (delta, fingerprint) = buffer.encode_snapshot_delta_with_native(
        Some(&fingerprint),
        &snap,
        NativeFactsInput {
            main_modal_texts: Some(MainModalTextsInput {
                root: 77,
                texts: &texts_b,
            }),
            ..Default::default()
        },
        false,
    );
    let wire = script::isolate_fb::decode_snapshot(&delta).unwrap();
    assert!(wire.has_main_modal_texts(), "the changed pair must post");
    assert_eq!(
        wire.main_modal_texts().expect("pair").texts(),
        vec!["@red@line a", "line b"]
    );
    assert!(wire.has_main_modal_id(), "the pair bit co-posts slot 68");
    assert_eq!(wire.main_modal_id(), 77);
    let value = tick(&iso, delta, 2);
    assert_eq!(
        value["pair"],
        serde_json::json!({ "root": 77, "texts": ["@red@line a", "line b"] }),
        "{value:?}"
    );
    assert_eq!(value["mainModalId"], 77, "{value:?}");

    // Observed closed: a present object, root -1 and empty lines. The
    // table's root wins over the snapshot's own `main_modal_id` in the same
    // buffer (the live path builds both from one rebuild, so they agree;
    // this pins the direction of the tie).
    snap.tick = 3;
    snap.main_modal_id = 77;
    let (delta, _) = buffer.encode_snapshot_delta_with_native(
        Some(&fingerprint),
        &snap,
        NativeFactsInput {
            main_modal_texts: Some(MainModalTextsInput {
                root: -1,
                texts: &[],
            }),
            ..Default::default()
        },
        false,
    );
    let value = tick(&iso, delta, 3);
    assert_eq!(value["hasPair"], true, "{value:?}");
    assert_eq!(
        value["pair"],
        serde_json::json!({ "root": -1, "texts": [] }),
        "a closed modal is a present object with no lines"
    );
    assert_eq!(value["mainModalId"], -1, "{value:?}");
    iso.join();
}

#[test]
fn a_first_post_without_the_pair_writes_no_property() {
    let iso = spawn_page_probe();
    // One-shot keyframe with no native facts: the pair was never supplied,
    // so the page has no property at all (not `null`, not the closed object).
    let value = tick(&iso, encode_snapshot(&base_snapshot()), 1);
    assert_eq!(value["hasPair"], false, "{value:?}");
    assert!(value["pair"].is_null(), "{value:?}");
    iso.join();
}

#[test]
fn changed_snapshot_clears_a_missing_quest_tab_to_unavailable() {
    let iso = spawn();
    let rows = [QuestStatusInput {
        name: "Waterfall Quest",
        status: "complete",
        component_id: Some(42),
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
fn a_rebuilt_row_with_a_new_id_re_posts_the_quest_tab() {
    let iso = spawn();
    let before = [QuestStatusInput {
        name: "Waterfall Quest",
        status: "complete",
        component_id: Some(42),
    }];
    // Same name and status, different walked id: the fingerprint carries the
    // id, so a quiet interface rebuild that moved the component still posts.
    let after = [QuestStatusInput {
        name: "Waterfall Quest",
        status: "complete",
        component_id: Some(84),
    }];
    let mut snap = base_snapshot();
    let mut buffer = IsolateBuf::new();
    let (keyframe, fingerprint) = buffer.encode_snapshot_delta_with_native(
        None,
        &snap,
        NativeFactsInput {
            quest_statuses: Some(&before),
            ..Default::default()
        },
        false,
    );
    assert_eq!(tick(&iso, keyframe, 1)["waterfall"], "complete");

    snap.tick = 2;
    let (delta, _) = buffer.encode_snapshot_delta_with_native(
        Some(&fingerprint),
        &snap,
        NativeFactsInput {
            quest_statuses: Some(&after),
            ..Default::default()
        },
        false,
    );
    let wire = script::isolate_fb::decode_snapshot(&delta).unwrap();
    assert!(
        wire.has_quest_statuses(),
        "a moved component id must re-post the rows"
    );
    assert_eq!(
        wire.quest_statuses()[0].component_id(),
        Some(84),
        "the row carries the new click target"
    );
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
        component_id: Some(42),
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
