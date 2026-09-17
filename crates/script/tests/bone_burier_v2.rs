//! Unit/integration coverage for the author-facing native v2 example.
//! These tests use synthetic isolate observations; they are not LIVE tests.

use std::path::PathBuf;

use script::isolate_fb::{
    BankApproachInput, BankStandInput, ItemRowInput, NativeFactsInput, ReachViewInput,
    SnapshotInput, StatInput, TileInput,
};
use script::load::{ApiFamily, JsLibrary, LoadIsolate, LoadShape};
use script::shim::InteractReq;

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

fn bone_stand() -> BankStandInput<'static> {
    BankStandInput {
        name: "Bank booth",
        x: 3200,
        z: 3200,
        level: 0,
        kind: "booth",
        op: 1,
        choose: None,
    }
}

fn bone_approach() -> [BankApproachInput; 1] {
    [BankApproachInput {
        loc_id: 1,
        x: 3200,
        z: 3200,
        level: 0,
        can_operate: true,
        dest_ok: true,
        dest_x: 3200,
        dest_z: 3200,
        dest_level: 0,
    }]
}

fn paint_burials(iso: &LoadIsolate) -> u64 {
    let paint = iso.paint().expect("paint forwarded");
    paint
        .lines
        .iter()
        .find_map(|line| {
            let rest = line.strip_prefix("burials | ")?;
            rest.parse().ok()
        })
        .unwrap_or(0)
}

fn count_bury_requests(iso: &LoadIsolate) -> usize {
    iso.drain_interacts()
        .into_iter()
        .filter(|req| matches!(req, InteractReq::Held { action, .. } if action == "Bury"))
        .count()
}

/// One bury at a time: request, unchanged snapshots must not duplicate, then observe loss.
fn simulate_serial_buries(
    iso: &LoadIsolate,
    facts: &NativeFactsInput<'_>,
    stand: &[BankStandInput],
    stats: &[StatInput],
    mut tick: u64,
    start: i32,
) -> (u64, usize) {
    let mut remaining = start;
    let mut bury_sends = 0;
    while remaining > 0 {
        let bones = [ItemRowInput::nc(Some("Bones"), remaining)];
        let mut s = base_snapshot();
        s.tick = tick;
        s.inv = &bones;
        s.banks = stand;
        s.stats = stats;
        post_tick(iso, s, *facts, tick);
        bury_sends += count_bury_requests(iso);
        for stall in 0..3 {
            tick += 1;
            let mut s = base_snapshot();
            s.tick = tick;
            s.inv = &bones;
            s.banks = stand;
            s.stats = stats;
            post_tick(iso, s, *facts, tick);
            assert_eq!(
                count_bury_requests(iso),
                0,
                "unchanged inventory must not resend bury (remaining={remaining}, stall={stall})"
            );
        }
        remaining -= 1;
        tick += 1;
        if remaining > 0 {
            let bones = [ItemRowInput::nc(Some("Bones"), remaining)];
            let mut s = base_snapshot();
            s.tick = tick;
            s.inv = &bones;
            s.banks = stand;
            s.stats = stats;
            post_tick(iso, s, *facts, tick);
            assert_eq!(
                count_bury_requests(iso),
                0,
                "completion tick must not immediately queue the next bury"
            );
        } else {
            let mut s = base_snapshot();
            s.tick = tick;
            s.inv = &[];
            s.banks = stand;
            s.stats = stats;
            post_tick(iso, s, *facts, tick);
            assert_eq!(count_bury_requests(iso), 0);
        }
    }
    (tick, bury_sends)
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
    assert!(iso.drain_interacts().is_empty());
    let mut s = base_snapshot();
    s.tick = 9;
    s.inv = &bones;
    s.banks = &stand;
    s.stats = &stats;
    post_tick(&iso, s, facts, 9);
    assert!(format!("{:?}", iso.drain_interacts()).contains("Bury"));
    let mut s = base_snapshot();
    s.tick = 10;
    s.inv = &unrelated;
    s.banks = &stand;
    s.stats = &stats;
    post_tick(&iso, s, facts, 10);
    assert!(iso.drain_interacts().is_empty());
    let mut s = base_snapshot();
    s.tick = 11;
    s.inv = &unrelated;
    s.banks = &stand;
    s.stats = &stats;
    post_tick(&iso, s, facts, 11);
    assert!(format!("{:?}", iso.drain_interacts()).contains("OpenStand"));
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
fn built_example_keeps_walk_nearest_bank_pending_past_short_op_bound() {
    let source = std::fs::read_to_string(example("bone_burier_v2.js")).unwrap();
    let iso = LoadIsolate::spawn(source, LoadShape::NativeTick, vec![]).unwrap();
    let filler = [ItemRowInput::nc(Some("Coins"), 1)];
    let stands = [BankStandInput {
        name: "Bank booth",
        x: 3200,
        z: 3200,
        level: 0,
        kind: "booth",
        op: 1,
        choose: None,
    }];
    let approaches = [BankApproachInput {
        loc_id: 1,
        x: 3210,
        z: 3210,
        level: 0,
        can_operate: true,
        dest_ok: true,
        dest_x: 3210,
        dest_z: 3210,
        dest_level: 0,
    }];
    let walk_facts = || NativeFactsInput {
        bank_approaches: Some(&approaches),
        ..Default::default()
    };
    let mut s = base_snapshot();
    s.tick = 1;
    s.inv = &filler;
    s.banks = &stands;
    post_tick(&iso, s, walk_facts(), 1);
    assert_eq!(iso.drain_interacts(), vec![InteractReq::WalkNearestBank]);
    assert!(!iso.stopped());
    for tick in 2..=100 {
        let mut s = base_snapshot();
        s.tick = tick;
        s.inv = &filler;
        s.banks = &stands;
        post_tick(&iso, s, walk_facts(), tick);
        assert!(
            !iso.stopped(),
            "walk-nearest-bank must not hit the 12-tick op bound (failed at tick {tick})"
        );
        assert!(iso.drain_interacts().is_empty());
    }
    let mut s = base_snapshot();
    s.tick = 101;
    s.inv = &filler;
    s.banks = &stands;
    let facts = NativeFactsInput {
        bank_approaches: Some(&approaches),
        walk_outcome_seq: 1,
        walk_outcome_failed: false,
        ..Default::default()
    };
    post_tick(&iso, s, facts, 101);
    assert!(!iso.stopped());
    iso.join();
}

#[test]
fn built_example_walk_pending_fails_closed_after_extended_bound() {
    let source = std::fs::read_to_string(example("bone_burier_v2.js")).unwrap();
    let iso = LoadIsolate::spawn(source, LoadShape::NativeTick, vec![]).unwrap();
    let filler = [ItemRowInput::nc(Some("Coins"), 1)];
    let stands = [BankStandInput {
        name: "Bank booth",
        x: 3200,
        z: 3200,
        level: 0,
        kind: "booth",
        op: 1,
        choose: None,
    }];
    let approaches = [BankApproachInput {
        loc_id: 1,
        x: 3210,
        z: 3210,
        level: 0,
        can_operate: true,
        dest_ok: true,
        dest_x: 3210,
        dest_z: 3210,
        dest_level: 0,
    }];
    let walk_facts = || NativeFactsInput {
        bank_approaches: Some(&approaches),
        ..Default::default()
    };
    let mut s = base_snapshot();
    s.tick = 1;
    s.inv = &filler;
    s.banks = &stands;
    post_tick(&iso, s, walk_facts(), 1);
    assert_eq!(iso.drain_interacts(), vec![InteractReq::WalkNearestBank]);
    for tick in 2..=121 {
        let mut s = base_snapshot();
        s.tick = tick;
        s.inv = &filler;
        s.banks = &stands;
        post_tick(&iso, s, walk_facts(), tick);
        assert!(!iso.stopped(), "expected still walking at tick {tick}");
    }
    let mut s = base_snapshot();
    s.tick = 122;
    s.inv = &filler;
    s.banks = &stands;
    post_tick(&iso, s, walk_facts(), 122);
    assert_eq!(
        iso.script_stop_receipt().unwrap().reason,
        "stalled while walk; no observed completion"
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

#[test]
fn built_example_does_not_resend_bury_while_inventory_unchanged() {
    let source = std::fs::read_to_string(example("bone_burier_v2.js")).unwrap();
    let iso = LoadIsolate::spawn(source, LoadShape::NativeTick, vec![]).unwrap();
    let approach = bone_approach();
    let stand = [bone_stand()];
    let facts = NativeFactsInput {
        bank_approaches: Some(&approach),
        ..Default::default()
    };
    let stats = [StatInput {
        index: 5,
        name: "Prayer",
        xp: 100,
        base: 1,
        effective: 1,
    }];
    let bones = [ItemRowInput::nc(Some("Bones"), 3)];
    let mut s = base_snapshot();
    s.tick = 1;
    s.inv = &bones;
    s.banks = &stand;
    s.stats = &stats;
    post_tick(&iso, s, facts, 1);
    assert_eq!(count_bury_requests(&iso), 1);
    for tick in 2..=5 {
        let mut s = base_snapshot();
        s.tick = tick;
        s.inv = &bones;
        s.banks = &stand;
        s.stats = &stats;
        post_tick(&iso, s, facts, tick);
        assert_eq!(count_bury_requests(&iso), 0, "tick {tick}");
    }
    iso.join();
}

#[test]
fn built_example_bury_pending_times_out_from_original_send_tick() {
    let source = std::fs::read_to_string(example("bone_burier_v2.js")).unwrap();
    let iso = LoadIsolate::spawn(source, LoadShape::NativeTick, vec![]).unwrap();
    let bones = [ItemRowInput::nc(Some("Bones"), 1)];
    let mut s = base_snapshot();
    s.tick = 1;
    s.inv = &bones;
    post_tick(&iso, s, NativeFactsInput::default(), 1);
    assert_eq!(count_bury_requests(&iso), 1);
    for tick in 2..=13 {
        let mut s = base_snapshot();
        s.tick = tick;
        s.inv = &bones;
        post_tick(&iso, s, NativeFactsInput::default(), tick);
        assert!(!iso.stopped(), "still waiting at tick {tick}");
        assert_eq!(count_bury_requests(&iso), 0);
    }
    let mut s = base_snapshot();
    s.tick = 14;
    s.inv = &bones;
    post_tick(&iso, s, NativeFactsInput::default(), 14);
    assert_eq!(
        iso.script_stop_receipt().unwrap().reason,
        "stalled while bury; no observed completion"
    );
    iso.join();
}

#[test]
fn built_example_delayed_inventory_loss_counts_exactly_once() {
    let source = std::fs::read_to_string(example("bone_burier_v2.js")).unwrap();
    let iso = LoadIsolate::spawn(source, LoadShape::NativeTick, vec![]).unwrap();
    let bones = [ItemRowInput::nc(Some("Bones"), 4)];
    let mut s = base_snapshot();
    s.tick = 1;
    s.inv = &bones;
    post_tick(&iso, s, NativeFactsInput::default(), 1);
    assert_eq!(count_bury_requests(&iso), 1);
    for tick in 2..=4 {
        let mut s = base_snapshot();
        s.tick = tick;
        s.inv = &bones;
        post_tick(&iso, s, NativeFactsInput::default(), tick);
        assert_eq!(count_bury_requests(&iso), 0);
    }
    let two = [ItemRowInput::nc(Some("Bones"), 2)];
    let mut s = base_snapshot();
    s.tick = 5;
    s.inv = &two;
    post_tick(&iso, s, NativeFactsInput::default(), 5);
    assert_eq!(count_bury_requests(&iso), 0);
    assert_eq!(paint_burials(&iso), 2);
    iso.join();
}

#[test]
fn built_example_open_pending_does_not_resend_while_bank_stays_closed() {
    let source = std::fs::read_to_string(example("bone_burier_v2.js")).unwrap();
    let iso = LoadIsolate::spawn(source, LoadShape::NativeTick, vec![]).unwrap();
    let approach = bone_approach();
    let stand = [bone_stand()];
    let facts = NativeFactsInput {
        bank_approaches: Some(&approach),
        ..Default::default()
    };
    let mut s = base_snapshot();
    s.tick = 1;
    s.banks = &stand;
    post_tick(&iso, s, facts, 1);
    assert!(format!("{:?}", iso.drain_interacts()).contains("OpenStand"));
    for tick in 2..=6 {
        let mut s = base_snapshot();
        s.tick = tick;
        s.banks = &stand;
        post_tick(&iso, s, facts, tick);
        assert!(iso.drain_interacts().is_empty(), "tick {tick}");
    }
    iso.join();
}

#[test]
fn built_example_load_pending_does_not_resend_while_inventory_unchanged() {
    let source = std::fs::read_to_string(example("bone_burier_v2.js")).unwrap();
    let iso = LoadIsolate::spawn(source, LoadShape::NativeTick, vec![]).unwrap();
    let bank_bones = [ItemRowInput::nc(Some("Bones"), 28)];
    let mut s = base_snapshot();
    s.tick = 1;
    s.bank_open = true;
    s.bank_loaded = true;
    s.bank_generation = 3;
    s.bank = &bank_bones;
    post_tick(&iso, s, NativeFactsInput::default(), 1);
    assert!(format!("{:?}", iso.drain_interacts()).contains("WithdrawLoad"));
    for tick in 2..=5 {
        let mut s = base_snapshot();
        s.tick = tick;
        s.bank_open = true;
        s.bank_loaded = true;
        s.bank_generation = 3;
        s.bank = &bank_bones;
        post_tick(&iso, s, NativeFactsInput::default(), tick);
        assert!(iso.drain_interacts().is_empty(), "tick {tick}");
    }
    iso.join();
}

#[test]
fn built_example_observes_thirty_three_burials_in_five_plus_twenty_eight_cycle() {
    let source = std::fs::read_to_string(example("bone_burier_v2.js")).unwrap();
    let iso = LoadIsolate::spawn(source, LoadShape::NativeTick, vec![]).unwrap();
    let approach = bone_approach();
    let stand = [bone_stand()];
    let facts = NativeFactsInput {
        bank_approaches: Some(&approach),
        ..Default::default()
    };
    let stats = [StatInput {
        index: 5,
        name: "Prayer",
        xp: 100,
        base: 1,
        effective: 1,
    }];
    let bank_supply = [ItemRowInput::nc(Some("Bones"), 28)];

    let (mut tick, mut bury_sends) =
        simulate_serial_buries(&iso, &facts, &stand, &stats, 1, 5);
    assert_eq!(bury_sends, 5);

    tick += 1;
    let mut s = base_snapshot();
    s.tick = tick;
    s.banks = &stand;
    s.stats = &stats;
    post_tick(&iso, s, facts, tick);
    assert!(format!("{:?}", iso.drain_interacts()).contains("OpenStand"));

    tick += 1;
    let mut s = base_snapshot();
    s.tick = tick;
    s.bank_open = true;
    s.bank_loaded = true;
    s.bank_generation = 7;
    s.bank = &bank_supply;
    s.stats = &stats;
    post_tick(&iso, s, facts, tick);

    tick += 1;
    let mut s = base_snapshot();
    s.tick = tick;
    s.bank_open = true;
    s.bank_loaded = true;
    s.bank_generation = 7;
    s.bank = &bank_supply;
    s.stats = &stats;
    post_tick(&iso, s, facts, tick);
    assert!(format!("{:?}", iso.drain_interacts()).contains("WithdrawLoad"));

    tick += 1;
    let loaded = [ItemRowInput::nc(Some("Bones"), 28)];
    let mut s = base_snapshot();
    s.tick = tick;
    s.inv = &loaded;
    s.bank_open = true;
    s.bank_loaded = true;
    s.bank_generation = 7;
    s.bank = &bank_supply;
    s.stats = &stats;
    post_tick(&iso, s, facts, tick);

    tick += 1;
    let mut s = base_snapshot();
    s.tick = tick;
    s.inv = &loaded;
    s.bank_open = true;
    s.bank_loaded = true;
    s.bank_generation = 7;
    s.bank = &bank_supply;
    s.stats = &stats;
    post_tick(&iso, s, facts, tick);
    assert!(format!("{:?}", iso.drain_interacts()).contains("Close"));

    tick += 1;
    let mut s = base_snapshot();
    s.tick = tick;
    s.inv = &loaded;
    s.stats = &stats;
    post_tick(&iso, s, facts, tick);
    assert!(iso.drain_interacts().is_empty());

    tick += 1;
    let (end_tick, bank_bury_sends) =
        simulate_serial_buries(&iso, &facts, &stand, &stats, tick, 28);
    tick = end_tick;
    bury_sends += bank_bury_sends;
    assert_eq!(bury_sends, 33);
    assert_eq!(paint_burials(&iso), 33);

    tick += 1;
    let mut s = base_snapshot();
    s.tick = tick;
    s.banks = &stand;
    s.stats = &stats;
    post_tick(&iso, s, facts, tick);
    assert!(format!("{:?}", iso.drain_interacts()).contains("OpenStand"));
    tick += 1;
    let mut s = base_snapshot();
    s.tick = tick;
    s.bank_open = true;
    s.bank_loaded = true;
    s.bank_generation = 8;
    s.bank = &[];
    s.stats = &stats;
    post_tick(&iso, s, facts, tick);
    tick += 1;
    let mut s = base_snapshot();
    s.tick = tick;
    s.bank_open = true;
    s.bank_loaded = true;
    s.bank_generation = 8;
    s.bank = &[];
    s.stats = &stats;
    post_tick(&iso, s, facts, tick);
    assert_eq!(
        iso.script_stop_receipt().unwrap().reason,
        "confirmed loaded current-generation bank exhaustion"
    );
    iso.join();
}
