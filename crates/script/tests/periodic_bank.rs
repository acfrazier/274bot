use script::isolate_fb::{
    encode_snapshot_with_native, BankApproachInput, ItemRowInput, NativeFactsInput,
    NearestBoothInput, ReachViewInput, SnapshotInput, TileInput,
};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};

fn post_snapshot_input(iso: &LoadIsolate, input: &SnapshotInput<'_>) {
    iso.post_snapshot(script::isolate_fb::encode_snapshot(input));
}

fn post_snapshot_native(
    iso: &LoadIsolate,
    input: &SnapshotInput<'_>,
    approaches: &[BankApproachInput],
) {
    iso.post_snapshot(encode_snapshot_with_native(
        input,
        NativeFactsInput {
            bank_approaches: Some(approaches),
            ..Default::default()
        },
    ));
}

fn approach_row(
    loc_id: i32,
    x: i32,
    z: i32,
    can_operate: bool,
    dest: Option<(i32, i32)>,
) -> BankApproachInput {
    BankApproachInput {
        loc_id,
        x,
        z,
        level: 0,
        can_operate,
        dest_ok: dest.is_some(),
        dest_x: dest.map(|tile| tile.0).unwrap_or(0),
        dest_z: dest.map(|tile| tile.1).unwrap_or(0),
        dest_level: 0,
    }
}

fn seers_ready_approach() -> BankApproachInput {
    approach_row(2213, 2725, 3490, true, Some((2724, 3490)))
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
fn loot_count_chicken_shape_observes_deposit_afterdeposit_and_return() {
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
    post_snapshot_native(&iso, &snap, &[seers_ready_approach()]);
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
    assert!(
        iso.drain_interacts().is_empty(),
        "frozen bankNearest waits one tick after afterDeposit"
    );

    snap.tick = 4;
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
            allow_wilderness: true,
            allow_bank_fetch: true,
            request_id: 0,
        }],
        "frozen bankNearest never closes the bank: the walk back starts next"
    );

    // The walk leaves the bank; the player arrives at the return tile.
    snap.tick = 5;
    snap.bank_open = false;
    snap.bank_loaded = false;
    snap.here = Some(TileInput {
        x: 3222,
        z: 3222,
        level: 0,
    });
    post_snapshot_input(&iso, &snap);
    tick(&iso, 5);
    assert!(iso.drain_interacts().is_empty());
    assert_eq!(iso.probe("__log").unwrap(), "periodic bank: completed");
    assert_eq!(iso.probe("__status").unwrap(), "periodic bank run");
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
    post_snapshot_native(&iso, &snap, &[seers_ready_approach()]);
    tick(&iso, 1);
    assert_eq!(iso.probe("__shim").unwrap(), "loot");
    assert!(matches!(
        iso.drain_interacts().as_slice(),
        [InteractReq::OpenBooth { .. }]
    ));
    iso.join();
}

/// `validate` is one pass over the options in the frozen order (strategy
/// twice, then the counts), and `execute` does not validate again: it reads
/// status, destination, commonJunk and returnTo, then banks.
#[test]
fn validate_and_execute_call_the_options_in_frozen_order() {
    let src = r#"
import { PeriodicBank } from '../../api/tasks/PeriodicBank.js';
export default class T extends TaskBot {
    onStart() {
        globalThis.__calls = [];
        const note = (name, value) => () => { globalThis.__calls.push(name); return value; };
        this.add(new PeriodicBank({
            strategy: note('strategy', 'loot'),
            itemsThreshold: note('itemsThreshold', 15),
            minutesThreshold: note('minutesThreshold', 10),
            countLoot: note('countLoot', 15),
            deposit: () => true,
            destination: note('destination', null),
            commonJunk: note('commonJunk', undefined),
            returnTo: note('returnTo', null),
            setStatus: note('setStatus'),
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
    post_snapshot_native(&iso, &snap, &[seers_ready_approach()]);
    tick(&iso, 1);
    assert_eq!(
        iso.probe("__calls").unwrap(),
        serde_json::json!([
            "strategy",
            "strategy",
            "countLoot",
            "itemsThreshold",
            "minutesThreshold",
            "setStatus",
            "destination",
            "commonJunk",
            "returnTo",
        ])
    );
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
            allow_wilderness: true,
            allow_bank_fetch: true,
            request_id: 0,
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
    post_snapshot_native(&iso, &snap, &[seers_ready_approach()]);
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

#[test]
fn chicken_killer_closed_face_walks_approach_dest_before_named_open_booth() {
    let iso = LoadIsolate::spawn(CHICKEN.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut bag = serde_json::Map::new();
    bag.insert("bankStrategy".into(), serde_json::json!("Loot count"));
    iso.post_settings_bag(&bag);

    let booth = NearestBoothInput {
        x: 3011,
        z: 3354,
        level: 0,
        id: 2213,
        name: "Bank booth",
        op: "Use-quickly",
    };
    let mut snap = base_snapshot();
    snap.here = Some(TileInput {
        x: 3010,
        z: 3355,
        level: 0,
    });
    snap.nearest_booth = Some(booth);
    let approaching = [approach_row(2213, 3011, 3354, false, Some((3011, 3355)))];
    post_snapshot_native(&iso, &snap, &approaching);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::WalkNear {
            x: 3011,
            z: 3355,
            level: 0,
            radius: 0,
            allow_teleports: false,
            allow_wilderness: true,
            allow_bank_fetch: true,
            request_id: 0,
        }],
        "Chebyshev 1 with can_operate false must not OpenBooth across a closed-face row"
    );

    snap.tick = 2;
    snap.here = Some(TileInput {
        x: 3011,
        z: 3355,
        level: 0,
    });
    let ready = [approach_row(2213, 3011, 3354, true, Some((3011, 3355)))];
    post_snapshot_native(&iso, &snap, &ready);
    tick(&iso, 2);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::OpenBooth {
            x: 3011,
            z: 3354,
            level: 0,
            id: 2213,
            name: Some("Bank booth".into()),
            action: Some("Use-quickly".into()),
        }]
    );
    iso.join();
}

#[test]
fn approach_dest_late_can_operate_opens_without_bank_generation_advance() {
    let iso = LoadIsolate::spawn(CHICKEN.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut bag = serde_json::Map::new();
    bag.insert("bankStrategy".into(), serde_json::json!("Loot count"));
    iso.post_settings_bag(&bag);

    const GEN: u64 = 12;
    let booth = NearestBoothInput {
        x: 3011,
        z: 3354,
        level: 0,
        id: 2213,
        name: "Bank booth",
        op: "Use-quickly",
    };
    let mut snap = base_snapshot();
    snap.bank_generation = GEN;
    snap.here = Some(TileInput {
        x: 3010,
        z: 3355,
        level: 0,
    });
    snap.nearest_booth = Some(booth);
    let approaching = [approach_row(2213, 3011, 3354, false, Some((3011, 3355)))];
    post_snapshot_native(&iso, &snap, &approaching);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::WalkNear {
            x: 3011,
            z: 3355,
            level: 0,
            radius: 0,
            allow_teleports: false,
            allow_wilderness: true,
            allow_bank_fetch: true,
            request_id: 0,
        }]
    );

    snap.tick = 2;
    snap.here = Some(TileInput {
        x: 3011,
        z: 3355,
        level: 0,
    });
    snap.bank_generation = GEN;
    let still_blocked = [approach_row(2213, 3011, 3354, false, Some((3011, 3355)))];
    post_snapshot_native(&iso, &snap, &still_blocked);
    tick(&iso, 2);
    assert!(
        iso.drain_interacts().is_empty(),
        "must wait on Rust approach readiness, not bank generation"
    );

    snap.tick = 3;
    snap.bank_generation = GEN;
    let ready = [approach_row(2213, 3011, 3354, true, Some((3011, 3355)))];
    post_snapshot_native(&iso, &snap, &ready);
    tick(&iso, 3);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::OpenBooth {
            x: 3011,
            z: 3354,
            level: 0,
            id: 2213,
            name: Some("Bank booth".into()),
            action: Some("Use-quickly".into()),
        }]
    );
    iso.join();
}

/// `Banking.bankNearest` with a chest destination (Shantay) walks within 4
/// of the bank tile, opens the chest through the access opener, and
/// deposits through the caller's matcher once the list posts.
#[test]
fn bank_nearest_chest_destination_opens_through_the_access_row() {
    let src = r#"
import { Banking } from '../../api/bank/Banking.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = await Banking.bankNearest({
            deposit: (name) => name === 'Coins',
            commonJunk: false,
            destination: {
                name: 'Shantay Pass',
                tile: { x: 3308, z: 3120, level: 0 },
                access: { name: 'Shantay chest', op: 'Open' },
            },
        });
    }
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    let actions = ["Open".to_string()];
    let chest = script::isolate_fb::SceneEntityInput {
        index: 0,
        id: 2693,
        name: Some("Shantay chest"),
        x: 3309,
        z: 3120,
        level: 0,
        distance: 1,
        health: -1,
        max_health: -1,
        in_combat: false,
        animating: false,
        actions: &actions,
        reachable: true,
        reachable_adj: true,
        combat_level: 0,
        target_kind: 0,
        target_index: -1,
        size: 0,
        nx: 0,
        nz: 0,
    };
    let locs = [chest];
    let mut snap = base_snapshot();
    snap.here = Some(TileInput {
        x: 3308,
        z: 3120,
        level: 0,
    });
    snap.locs = &locs;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Loc {
            x: 3309,
            z: 3120,
            level: 0,
            action: "Open".into(),
            id: Some(2693),
        }]
    );

    let side = [item_row("Coins", 995, 25)];
    snap.tick = 2;
    snap.bank_open = true;
    snap.bank_loaded = true;
    snap.bank_generation = 1;
    snap.bank_side = &side;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Deposit {
            name: "Coins".into()
        }]
    );
    iso.join();
}

/// Frozen calls `setStatus`, `log` and the deposit matcher synchronously: a
/// never-settling status promise does not hold the run, and an async
/// matcher's Promise is truthy, so the first backpack row is deposited.
#[test]
fn synchronous_options_are_not_awaited() {
    let src = r#"
import { PeriodicBank } from '../../api/tasks/PeriodicBank.js';
export default class T extends TaskBot {
    onStart() {
        this.add(new PeriodicBank({
            strategy: () => 'loot',
            itemsThreshold: () => 1,
            minutesThreshold: () => 10,
            countLoot: () => 2,
            deposit: async (name) => name === 'Coins',
            commonJunk: () => false,
            returnTo: () => null,
            setStatus: () => new Promise(() => {}),
            log: () => new Promise(() => {}),
        }));
    }
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    let side = [item_row("Bones", 526, 1), item_row("Coins", 995, 25)];
    let mut snap = base_snapshot();
    snap.here = Some(TileInput {
        x: 2724,
        z: 3490,
        level: 0,
    });
    snap.nearest_booth = Some(seers_booth());
    snap.bank_open = true;
    snap.bank_loaded = true;
    snap.bank_generation = 4;
    snap.bank_side = &side;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Deposit {
            name: "Bones".into()
        }]
    );
    iso.join();
}

/// A booth beside the player with no posted approach fact is not opened:
/// the run fails closed and backs off with the frozen log line.
#[test]
fn an_adjacent_booth_without_an_approach_fact_fails_closed() {
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
    post_snapshot_native(&iso, &snap, &[]);
    tick(&iso, 1);
    assert!(
        iso.drain_interacts().is_empty(),
        "no OpenBooth without a fact"
    );
    assert_eq!(
        iso.probe("__log").unwrap(),
        "periodic bank: no bank reachable — will retry later"
    );
    iso.join();
}
