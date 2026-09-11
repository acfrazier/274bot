use script::isolate_fb::{
    ItemRowInput, NearestBoothInput, ReachViewInput, SnapshotInput, TileInput,
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
            x: 3222,
            z: 3222,
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
        side_tab: -1,
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
    }
}

fn item_row<'a>(name: &'a str, id: i32, count: i32) -> ItemRowInput<'a> {
    ItemRowInput {
        name: Some(name),
        count,
        id,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: 0,
        slot: -1,
    }
}

fn seers_booth() -> NearestBoothInput<'static> {
    NearestBoothInput {
        x: 2725,
        z: 3490,
        level: 0,
        id: 2213,
        name: "Bank booth",
        op: "Use-quickly",
    }
}

fn tick(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    let _ = iso.probe("true");
}

const CHICKEN: &str = r#"
import { PeriodicBank } from '../../api/tasks/PeriodicBank.js';
import { depositAllExcept, parseBankStrategy } from '../../api/bank/Banking.js';
export default class ChickenShaped extends TaskBot {
    onStart() {
        this.add(new PeriodicBank({
            strategy: () => parseBankStrategy(this.settings.str('bankStrategy', 'Off')),
            itemsThreshold: () => this.settings.num('bankEveryItems', 15),
            minutesThreshold: () => this.settings.num('bankEveryMinutes', 10),
            countLoot: () => 15,
            deposit: depositAllExcept(['Bronze arrow', 'Mind rune']),
            afterDeposit: () => { globalThis.__after = true; },
            destination: () => null,
            returnTo: () => ({ x: 3222, z: 3222, level: 0 }),
            setStatus: (s) => { globalThis.__status = s; },
            log: (m) => { globalThis.__log = m; },
        }));
    }
}
"#;

#[test]
fn off_periodic_bank_validates_false_and_sends_nothing() {
    let iso = LoadIsolate::spawn(CHICKEN.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut bag = serde_json::Map::new();
    bag.insert("bankStrategy".into(), serde_json::json!("Off"));
    iso.post_settings_bag(&bag);
    let mut snap = base_snapshot();
    snap.nearest_booth = Some(seers_booth());
    snap.here = Some(TileInput {
        x: 2724,
        z: 3490,
        level: 0,
    });
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    tick(&iso, 2);
    assert!(
        iso.drain_interacts().is_empty(),
        "Off must not queue open/deposit/walk"
    );
    iso.join();
}

#[test]
fn combat_suppresses_loot_count_trigger() {
    let iso = LoadIsolate::spawn(CHICKEN.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut bag = serde_json::Map::new();
    bag.insert("bankStrategy".into(), serde_json::json!("Loot count"));
    iso.post_settings_bag(&bag);
    let mut snap = base_snapshot();
    snap.in_combat = true;
    snap.nearest_booth = Some(seers_booth());
    snap.here = Some(TileInput {
        x: 2724,
        z: 3490,
        level: 0,
    });
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert!(
        iso.drain_interacts().is_empty(),
        "combat suppresses banking"
    );
    iso.join();
}

#[test]
fn loot_count_chicken_shape_observes_deposit_afterdeposit_close_and_return() {
    let iso = LoadIsolate::spawn(CHICKEN.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut bag = serde_json::Map::new();
    bag.insert("bankStrategy".into(), serde_json::json!("Loot count"));
    bag.insert("bankEveryItems".into(), serde_json::json!(15));
    iso.post_settings_bag(&bag);

    let booth = seers_booth();
    let mut snap = base_snapshot();
    snap.here = Some(TileInput {
        x: 2724,
        z: 3490,
        level: 0,
    });
    snap.nearest_booth = Some(booth);
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::OpenBooth {
            x: 2725,
            z: 3490,
            level: 0,
            id: 2213,
            name: Some("Bank booth".into()),
            action: Some("Use-quickly".into()),
        }],
        "queued open is not completion; identity is the adjacent booth"
    );
    assert!(
        iso.probe("__after").is_err(),
        "afterDeposit must not run before a fresh snapshot"
    );

    let bones = [item_row("Bones", 526, 1), item_row("Bronze arrow", 882, 50)];
    snap.tick = 2;
    snap.bank_open = true;
    snap.bank_loaded = true;
    snap.bank_generation = 1;
    snap.bank_side = &bones;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Deposit {
            name: "Bones".into()
        }]
    );
    assert!(
        iso.probe("__after").is_err(),
        "queued deposit is not completion"
    );

    let kept = [item_row("Bronze arrow", 882, 50)];
    snap.tick = 3;
    snap.bank_side = &kept;
    snap.bank_op_result_seq = 1;
    snap.bank_op_result = true;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 3);
    assert_eq!(
        iso.probe("__after").unwrap(),
        true,
        "afterDeposit runs after observed deposit"
    );
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Close],
        "Rust must wait for the script callback before close"
    );

    snap.tick = 4;
    snap.bank_open = false;
    snap.bank_loaded = false;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 4);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::WalkNear {
            x: 3222,
            z: 3222,
            level: 0,
            radius: 6,
            allow_teleports: false,
        }]
    );

    snap.tick = 5;
    snap.here = Some(TileInput {
        x: 3222,
        z: 3222,
        level: 0,
    });
    post_snapshot_input(&iso, &snap);
    tick(&iso, 5);
    assert!(iso.drain_interacts().is_empty());
    assert_eq!(iso.probe("__log").unwrap(), "periodic bank: completed");
    iso.join();
}

#[test]
fn either_label_and_shim_loot_token_both_trigger() {
    let src = r#"
import { PeriodicBank } from '../../api/tasks/PeriodicBank.js';
import { parseBankStrategy } from '../../api/bank/Banking.js';
export default class T extends TaskBot {
    onStart() {
        globalThis.__shim = parseBankStrategy('Loot count');
        this.add(new PeriodicBank({
            strategy: () => parseBankStrategy('Either'),
            itemsThreshold: () => 15,
            minutesThreshold: () => 10,
            countLoot: () => 15,
            deposit: () => true,
            returnTo: () => null,
        }));
    }
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = base_snapshot();
    snap.here = Some(TileInput {
        x: 2724,
        z: 3490,
        level: 0,
    });
    snap.nearest_booth = Some(seers_booth());
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__shim").unwrap(), "loot");
    assert!(matches!(
        iso.drain_interacts().as_slice(),
        [InteractReq::OpenBooth { .. }]
    ));
    iso.join();
}

#[test]
fn supplied_destination_is_exact_and_does_not_open_a_different_loc() {
    let src = r#"
import { PeriodicBank } from '../../api/tasks/PeriodicBank.js';
export default class T extends TaskBot {
    onStart() {
        this.add(new PeriodicBank({
            strategy: () => 'loot',
            itemsThreshold: () => 1,
            minutesThreshold: () => 10,
            countLoot: () => 3,
            deposit: (name) => name === 'Coins',
            destination: () => ({ tile: { x: 2725, z: 3490, level: 0 } }),
            returnTo: () => null,
        }));
    }
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = base_snapshot();
    snap.here = Some(TileInput {
        x: 2600,
        z: 3400,
        level: 0,
    });
    snap.nearest_booth = Some(NearestBoothInput {
        x: 2655,
        z: 3286,
        level: 0,
        id: 2213,
        name: "Bank booth",
        op: "Use-quickly",
    });
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::WalkNear {
            x: 2725,
            z: 3490,
            level: 0,
            radius: 1,
            allow_teleports: false,
        }],
        "RockCrab-shaped bankTile must walk the supplied stand"
    );

    snap.tick = 2;
    snap.here = Some(TileInput {
        x: 2725,
        z: 3490,
        level: 0,
    });
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    let reqs = iso.drain_interacts();
    assert!(
        reqs.iter()
            .all(|req| !matches!(req, InteractReq::OpenBooth { x: 2655, .. })),
        "must not fall back to a different booth: {reqs:?}"
    );
    iso.join();
}

#[test]
fn common_junk_true_deposits_gem_false_skips_it() {
    let src = r#"
import { PeriodicBank } from '../../api/tasks/PeriodicBank.js';
export default class T extends TaskBot {
    onStart() {
        this.add(new PeriodicBank({
            strategy: () => 'loot',
            itemsThreshold: () => 1,
            minutesThreshold: () => 10,
            countLoot: () => 2,
            deposit: (name) => name === 'Coins',
            commonJunk: () => globalThis.__junk === true,
            returnTo: () => null,
        }));
    }
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    iso.probe("globalThis.__junk = true").unwrap();
    let booth = seers_booth();
    let side = [
        item_row("Uncut sapphire", 1623, 1),
        item_row("Coins", 995, 25),
    ];
    let mut snap = base_snapshot();
    snap.here = Some(TileInput {
        x: 2724,
        z: 3490,
        level: 0,
    });
    snap.nearest_booth = Some(booth);
    snap.bank_open = true;
    snap.bank_loaded = true;
    snap.bank_generation = 4;
    snap.bank_side = &side;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Deposit {
            name: "Uncut sapphire".into()
        }]
    );
    iso.join();

    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    iso.probe("globalThis.__junk = false").unwrap();
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Deposit {
            name: "Coins".into()
        }]
    );
    iso.join();
}

#[test]
fn missing_access_backs_off_without_retry_spam() {
    let iso = LoadIsolate::spawn(CHICKEN.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut bag = serde_json::Map::new();
    bag.insert("bankStrategy".into(), serde_json::json!("Loot count"));
    iso.post_settings_bag(&bag);
    let snap = base_snapshot();
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert!(iso.drain_interacts().is_empty());
    assert!(iso
        .probe("__log")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("no bank reachable"));
    tick(&iso, 2);
    tick(&iso, 3);
    assert!(
        iso.drain_interacts().is_empty(),
        "backoff must not retry-spam inside 180s"
    );
    iso.join();
}

#[test]
fn pause_and_session_reset_drop_late_callback_and_sends() {
    let iso = LoadIsolate::spawn(CHICKEN.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut bag = serde_json::Map::new();
    bag.insert("bankStrategy".into(), serde_json::json!("Loot count"));
    iso.post_settings_bag(&bag);
    let mut snap = base_snapshot();
    snap.here = Some(TileInput {
        x: 2724,
        z: 3490,
        level: 0,
    });
    snap.nearest_booth = Some(seers_booth());
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert!(matches!(
        iso.drain_interacts().as_slice(),
        [InteractReq::OpenBooth { .. }]
    ));

    iso.pause();
    snap.tick = 2;
    snap.bank_open = true;
    snap.bank_loaded = true;
    snap.bank_generation = 9;
    let bones = [item_row("Bones", 526, 1)];
    snap.bank_side = &bones;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert!(
        iso.drain_interacts().is_empty(),
        "Pause/hold must not send deposit while frozen"
    );
    assert!(iso.probe("__after").is_err());

    iso.resume();
    tick(&iso, 3);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Deposit {
            name: "Bones".into()
        }]
    );

    iso.reset_session_work();
    snap.tick = 4;
    snap.bank_op_result_seq = 1;
    snap.bank_op_result = true;
    snap.bank_side = &[];
    post_snapshot_input(&iso, &snap);
    tick(&iso, 4);
    assert!(
        iso.probe("__after").is_err(),
        "session replacement must not run the old afterDeposit"
    );
    assert!(
        iso.drain_interacts()
            .iter()
            .all(|req| !matches!(req, InteractReq::Close)),
        "old close must not land after reset"
    );
    iso.join();
}
