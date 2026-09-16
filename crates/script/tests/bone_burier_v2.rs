//! Unit/integration coverage for the author-facing native v2 example.
//! These tests use synthetic isolate observations; they are not LIVE tests.

use std::path::PathBuf;

use script::isolate_fb::{
    BankApproachInput, BankStandInput, ItemRowInput, NativeFactsInput, ReachViewInput,
    SnapshotInput, StatInput, TileInput,
};
use script::load::{ApiFamily, JsLibrary, LoadIsolate, LoadShape};

fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join(name)
}

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("274bot-bone-burier-v2-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn base_snapshot<'a>() -> SnapshotInput<'a> {
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
        hold: false,
        ours: false,
        npcs: &[],
        withdraw_load_result_seq: 0,
        withdraw_load_result: false,
        bank_op_result_seq: 0,
        bank_op_result: false,
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

fn post_tick(iso: &LoadIsolate, snap: SnapshotInput<'_>, facts: NativeFactsInput<'_>, tick: u64) {
    iso.post_snapshot(script::isolate_fb::encode_snapshot_with_native(
        &snap, facts,
    ));
    iso.on_game_tick(tick);
    let _ = iso.probe("true");
}

#[test]
fn authoritative_typescript_and_built_javascript_load_as_v2() {
    let dir = scratch();
    let mut library = JsLibrary::with_cache(dir.join("cards.json"), dir.join("cache"));
    let ts_path = example("bone_burier_v2.ts");
    let js_path = example("bone_burier_v2.js");
    let ts = library.load(&ts_path).unwrap();
    let js = library.load(&js_path).unwrap();

    assert_eq!(ts.shape, LoadShape::NativeTick);
    assert_eq!(ts.api_family, ApiFamily::V2);
    assert_eq!(js.shape, LoadShape::NativeTick);
    assert_eq!(js.api_family, ApiFamily::V2);
    assert!(ts.unloadable.is_none(), "unexpected TS import failure");
    assert!(js.unloadable.is_none(), "unexpected JS import failure");
    assert_eq!(ts.settings_schema.len(), 1);
    assert_eq!(ts.settings_schema[0].id, "boneName");
    assert!(js.js.contains("apiVersion"));
}

#[test]
fn built_example_stops_cleanly_without_snapshot_and_settings_are_isolated() {
    let source = std::fs::read_to_string(example("bone_burier_v2.js")).unwrap();
    let left = LoadIsolate::spawn(source.clone(), LoadShape::NativeTick, vec![]).unwrap();
    let right = LoadIsolate::spawn(source, LoadShape::NativeTick, vec![]).unwrap();
    let mut left_settings = serde_json::Map::new();
    left_settings.insert("boneName".into(), serde_json::json!("Dragon bones"));
    left.post_settings_bag(&left_settings);
    let mut right_settings = serde_json::Map::new();
    right_settings.insert("boneName".into(), serde_json::json!("Bones"));
    right.post_settings_bag(&right_settings);
    assert_eq!(
        left.probe("globalThis.__rs_api.settings.str('boneName')")
            .unwrap(),
        "Dragon bones"
    );
    assert_eq!(
        right
            .probe("globalThis.__rs_api.settings.str('boneName')")
            .unwrap(),
        "Bones"
    );
    let mut left_snap = base_snapshot();
    left_snap.ingame = false;
    let mut right_snap = base_snapshot();
    right_snap.ingame = false;
    post_tick(&left, left_snap, NativeFactsInput::default(), 1);
    post_tick(&right, right_snap, NativeFactsInput::default(), 1);
    assert_eq!(left.script_stop_receipt().unwrap().reason, "not in game");
    assert_eq!(right.script_stop_receipt().unwrap().reason, "not in game");
    assert!(left.stopped() && right.stopped());
    left.join();
    right.join();
}

#[test]
fn built_example_observes_bury_restock_cycle_then_loaded_exhaustion() {
    let source = std::fs::read_to_string(example("bone_burier_v2.js")).unwrap();
    let iso = LoadIsolate::spawn(source, LoadShape::NativeTick, vec![]).unwrap();
    let bones = [ItemRowInput::nc(Some("Bones"), 2)];
    let unrelated = [ItemRowInput::nc(Some("Coins"), 10)];
    let bank_bones = [ItemRowInput::nc(Some("Bones"), 2)];
    let stand = [BankStandInput {
        name: "Bank booth",
        x: 3200,
        z: 3200,
        level: 0,
        kind: "booth",
        op: 1,
        choose: None,
    }];
    let approach = [BankApproachInput {
        loc_id: 1,
        x: 3200,
        z: 3200,
        level: 0,
        can_operate: true,
        dest_ok: true,
        dest_x: 3200,
        dest_z: 3200,
        dest_level: 0,
    }];
    let stats = [StatInput {
        index: 5,
        name: "Prayer",
        xp: 100,
        base: 1,
        effective: 1,
    }];
    let facts = NativeFactsInput {
        bank_approaches: Some(&approach),
        ..Default::default()
    };
    let mut s = base_snapshot();
    s.inv = &bones;
    s.banks = &stand;
    s.stats = &stats;
    post_tick(&iso, s, facts, 1);
    assert!(format!("{:?}", iso.drain_interacts()).contains("Bury"));
    let mut s = base_snapshot();
    s.tick = 2;
    s.inv = &unrelated;
    s.banks = &stand;
    s.stats = &stats;
    post_tick(&iso, s, facts, 2);
    let mut s = base_snapshot();
    s.tick = 3;
    s.inv = &unrelated;
    s.banks = &stand;
    s.stats = &stats;
    post_tick(&iso, s, facts, 3);
    let open = iso.drain_interacts();
    assert!(format!("{open:?}").contains("OpenStand"));
    let mut s = base_snapshot();
    s.tick = 4;
    s.bank_open = true;
    s.bank_loaded = true;
    s.bank_generation = 7;
    s.bank = &bank_bones;
    s.stats = &stats;
    post_tick(&iso, s, facts, 4);
    let mut s = base_snapshot();
    s.tick = 5;
    s.bank_open = true;
    s.bank_loaded = true;
    s.bank_generation = 7;
    s.bank = &bank_bones;
    s.stats = &stats;
    post_tick(&iso, s, facts, 5);
    assert!(format!("{:?}", iso.drain_interacts()).contains("WithdrawLoad"));
    let mut s = base_snapshot();
    s.tick = 6;
    s.inv = &bones;
    s.bank_open = true;
    s.bank_loaded = true;
    s.bank_generation = 7;
    s.bank = &bank_bones;
    s.stats = &stats;
    post_tick(&iso, s, facts, 6);
    let mut s = base_snapshot();
    s.tick = 7;
    s.inv = &bones;
    s.bank_open = true;
    s.stats = &stats;
    post_tick(&iso, s, facts, 7);
    assert!(format!("{:?}", iso.drain_interacts()).contains("Close"));
    let mut s = base_snapshot();
    s.tick = 8;
    s.inv = &bones;
    s.stats = &stats;
    post_tick(&iso, s, facts, 8);
    let mut s = base_snapshot();
    s.tick = 9;
    s.inv = &unrelated;
    s.banks = &stand;
    s.stats = &stats;
    post_tick(&iso, s, facts, 9);
    let mut s = base_snapshot();
    s.tick = 10;
    s.inv = &unrelated;
    s.banks = &stand;
    s.stats = &stats;
    post_tick(&iso, s, facts, 10);
    let mut s = base_snapshot();
    s.tick = 12;
    s.bank_open = true;
    s.bank_loaded = true;
    s.bank_generation = 8;
    s.stats = &stats;
    post_tick(&iso, s, facts, 12);
    let mut s = base_snapshot();
    s.tick = 13;
    s.bank_open = true;
    s.bank_loaded = true;
    s.bank_generation = 8;
    s.stats = &stats;
    post_tick(&iso, s, facts, 13);
    assert_eq!(
        iso.script_stop_receipt().unwrap().reason,
        "confirmed loaded current-generation bank exhaustion"
    );
    assert!(iso.stopped());
    iso.join();
}

#[test]
fn unloaded_or_missing_bank_is_not_exhaustion_and_eventually_stops() {
    let source = std::fs::read_to_string(example("bone_burier_v2.js")).unwrap();
    let iso = LoadIsolate::spawn(source, LoadShape::NativeTick, vec![]).unwrap();
    for tick in 1..15 {
        let mut s = base_snapshot();
        s.tick = tick;
        s.bank_open = true;
        s.bank_loaded = false;
        post_tick(&iso, s, NativeFactsInput::default(), tick);
    }
    let reason = iso.script_stop_receipt().unwrap().reason;
    assert!(iso.stopped());
    assert!(reason.contains("unavailable") || reason.contains("stalled"));
    assert!(!reason.contains("exhaustion"));
    iso.join();
}
