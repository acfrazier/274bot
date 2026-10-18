use std::sync::Arc;

mod common;
use common::bank_slot::BankSlot;
use script::isolate_fb::{
    ItemRowInput, NativeFactsInput, ReachViewInput, SnapshotInput, TileInput,
};
use script::native::{HostAction, HostEffect};
use script::shim::InteractReq;
use script::LoadShape;

fn spawn(source: impl Into<String>) -> BankSlot {
    spawn_with_facts(source, Arc::new(api::named_banks::NamedBankFacts::empty()))
}

fn spawn_with_facts(
    source: impl Into<String>,
    facts: Arc<api::named_banks::NamedBankFacts>,
) -> BankSlot {
    BankSlot::new(source, LoadShape::CompatClass, None, facts)
}
fn picker_facts() -> Arc<api::named_banks::NamedBankFacts> {
    Arc::new(api::named_banks::NamedBankFacts::from_banks(vec![
        api::named_banks::NamedBank::new(
            "Test bank",
            api::snapshot::WorldTile {
                x: 3000,
                z: 3000,
                level: 0,
            },
        ),
    ]))
}

fn spawn_with_settings(
    source: impl Into<String>,
    settings: &serde_json::Map<String, serde_json::Value>,
) -> BankSlot {
    BankSlot::new(
        source,
        LoadShape::CompatClass,
        Some(settings),
        Arc::new(api::named_banks::NamedBankFacts::empty()),
    )
}

fn take_interactions(slot: &mut BankSlot) -> Vec<InteractReq> {
    slot.take_actions()
        .into_iter()
        .map(|action| match action.effect {
            HostEffect::Interaction(request) => request,
            _ => panic!("expected a native interaction action"),
        })
        .collect()
}

trait BankSlotTestExt {
    fn drain_interacts(&mut self) -> Vec<InteractReq>;
    fn join(&mut self);
    fn post_settings_bag(&mut self, bag: &serde_json::Map<String, serde_json::Value>);
}

impl BankSlotTestExt for BankSlot {
    fn drain_interacts(&mut self) -> Vec<InteractReq> {
        take_interactions(self)
    }

    fn join(&mut self) {
        self.stop();
    }

    fn post_settings_bag(&mut self, bag: &serde_json::Map<String, serde_json::Value>) {
        self.slot.post_settings_bag(bag);
    }
}

/// The host raises the bank-side root with the open bank and drops it on close.
fn side_root(input: &SnapshotInput<'_>) -> i32 {
    if input.bank_open {
        SIDE_ROOT
    } else {
        -1
    }
}

fn post_snapshot_input(slot: &mut BankSlot, input: &SnapshotInput<'_>) {
    slot.post_snapshot(
        input,
        NativeFactsInput {
            bank_snapshot_generation: input
                .bank_open
                .then(|| i32::try_from(input.bank_generation).expect("small fixture generation")),
            side_modal_id: Some(side_root(input)),
            ..Default::default()
        },
    );
}

fn tick(slot: &mut BankSlot, tick: u64) {
    slot.tick_posted(tick);
}

fn one_action(slot: &mut BankSlot, message: &str) -> HostAction {
    let mut actions = slot.take_actions();
    assert_eq!(actions.len(), 1, "{message}");
    actions.remove(0)
}

fn accept_interaction(slot: &mut BankSlot, action: &HostAction, tick: u64) {
    let authority = action.authority();
    slot.slot.complete_native_interaction(
        &authority,
        script::native::InteractionReceipt {
            request_id: action.request_id.get(),
            evidence: api::quest_progress::EvidenceStamp {
                run: authority.run(),
                tick,
                sequence: tick,
            },
            accepted: true,
            chat_since: 0,
        },
    );
}

fn arrive_walk(slot: &mut BankSlot, action: &HostAction, tick: u64) {
    let authority = action.authority();
    slot.slot.complete_native_walk(
        &authority,
        script::native::WalkReceipt {
            request_id: action.request_id.get(),
            evidence: api::quest_progress::EvidenceStamp {
                run: authority.run(),
                tick,
                sequence: tick,
            },
            end: script::native::WalkEnd::Arrived,
            blocked: None,
            detail: None,
            refusal: None,
            assessment: None,
            escape: None,
        },
    );
}

fn no_bank_candidate(slot: &mut BankSlot, action: &HostAction, tick: u64) {
    let authority = action.authority();
    slot.slot.complete_native_bank_pick(
        &authority,
        script::native_bank::BankPickReceipt {
            request_id: action.request_id.get(),
            evidence: api::quest_progress::EvidenceStamp {
                run: authority.run(),
                tick,
                sequence: tick,
            },
            selected: script::native_bank::SelectedBank {
                bank_index: u16::MAX,
                access_tile: api::snapshot::WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                kind: script::native_bank::PickKind::NoCandidate,
                access: None,
            },
        },
    );
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
        user_move_intent_seq: 0,
        walk_outcome_cancel_reason: Default::default(),
    }
}

const SIDE_ROOT: i32 = 700;
const SIDE_GRID: i32 = 701;

/// The 289 deposit grid's ops.
static DEPOSIT_OPS: std::sync::LazyLock<Vec<String>> = std::sync::LazyLock::new(|| {
    [
        "Deposit-1",
        "Deposit-5",
        "Deposit-10",
        "Deposit-All",
        "Deposit-X",
    ]
    .map(String::from)
    .to_vec()
});

/// A bank-side row at `slot` of the deposit grid.
fn item_row<'a>(name: &'a str, id: i32, count: i32, slot: i32) -> ItemRowInput<'a> {
    ItemRowInput {
        name: Some(name),
        count,
        id,
        ops: &DEPOSIT_OPS,
        noted: false,
        cert: -1,
        component_id: SIDE_GRID,
        slot,
    }
}
fn pack_row<'a>(name: &'a str, id: i32, count: i32, slot: i32) -> ItemRowInput<'a> {
    ItemRowInput {
        ops: &[],
        component_id: 0,
        ..item_row(name, id, count, slot)
    }
}

/// Deposit-All of side row `id` at `slot` in bank session `generation`.
fn deposit_all(id: i32, slot: i32, generation: u64) -> InteractReq {
    InteractReq::InvButton {
        id,
        slot,
        component: SIDE_GRID,
        operation: 4,
        bank_generation: generation,
    }
}

fn seers_booth() -> &'static script::isolate_fb::SceneEntityInput<'static> {
    static BOOTH: std::sync::LazyLock<script::isolate_fb::SceneEntityInput<'static>> =
        std::sync::LazyLock::new(|| {
            static OPS: std::sync::LazyLock<Vec<String>> =
                std::sync::LazyLock::new(|| vec!["Use-quickly".to_owned()]);
            script::isolate_fb::SceneEntityInput {
                index: 0,
                id: 2213,
                name: Some("Bank booth"),
                x: 2725,
                z: 3490,
                level: 0,
                distance: 1,
                health: -1,
                max_health: -1,
                in_combat: false,
                animating: false,
                actions: &OPS,
                reachable: true,
                reachable_adj: true,
                combat_level: 0,
                target_kind: 0,
                target_index: -1,
                size: 1,
                nx: 2725,
                nz: 3490,
                shape: 10,
                angle: 0,
            }
        });
    &BOOTH
}

fn set_seers_scene(input: &mut SnapshotInput<'_>) {
    input.locs = std::slice::from_ref(seers_booth());
    input.reach = seers_reach();
}

fn seers_reach() -> ReachViewInput<'static> {
    ReachViewInput {
        available: true,
        base_x: 2724,
        base_z: 3490,
        level: 0,
        width: 2,
        height: 1,
        walkable: &[0b11],
        reachable: &[0b11],
        reachable_adj: &[0b11],
        exact_rank: &[0, 1],
        adjacent_rank: &[0, 1],
        step: &[0, 0],
        canlight: &[],
        stamp: 1,
    }
}
fn open_reach_at(base_x: i32, base_z: i32) -> ReachViewInput<'static> {
    let mut reach = seers_reach();
    reach.base_x = base_x;
    reach.base_z = base_z;
    reach
}
fn diagonal_approach_reach() -> ReachViewInput<'static> {
    ReachViewInput {
        available: true,
        base_x: 3010,
        base_z: 3354,
        level: 0,
        width: 2,
        height: 2,
        walkable: &[0b1010],
        reachable: &[0b1010],
        reachable_adj: &[0b1010],
        exact_rank: &[u16::MAX, 0, u16::MAX, 1],
        adjacent_rank: &[u16::MAX, 0, u16::MAX, 1],
        step: &[0, 0, 0, 0],
        canlight: &[],
        stamp: 1,
    }
}

fn stand_ready_reach() -> ReachViewInput<'static> {
    ReachViewInput {
        available: true,
        base_x: 3011,
        base_z: 3354,
        level: 0,
        width: 1,
        height: 2,
        walkable: &[0b11],
        reachable: &[0b10],
        reachable_adj: &[0b10],
        exact_rank: &[u16::MAX, 0],
        adjacent_rank: &[u16::MAX, 0],
        step: &[0, 0],
        canlight: &[],
        stamp: 2,
    }
}

fn booth_at(x: i32, z: i32) -> script::isolate_fb::SceneEntityInput<'static> {
    let mut booth = *seers_booth();
    booth.x = x;
    booth.z = z;
    booth.nx = x;
    booth.nz = z;
    booth
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
    let mut iso = spawn(CHICKEN);
    let mut bag = serde_json::Map::new();
    bag.insert("bankStrategy".into(), serde_json::json!("Off"));
    iso.post_settings_bag(&bag);
    let mut snap = base_snapshot();
    set_seers_scene(&mut snap);
    snap.here = Some(TileInput {
        x: 2724,
        z: 3490,
        level: 0,
    });
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 1);
    tick(&mut iso, 2);
    assert!(
        iso.drain_interacts().is_empty(),
        "Off must not queue open/deposit/walk"
    );
    iso.join();
}

#[test]
fn combat_suppresses_loot_count_trigger() {
    let mut iso = spawn(CHICKEN);
    let mut bag = serde_json::Map::new();
    bag.insert("bankStrategy".into(), serde_json::json!("Loot count"));
    iso.post_settings_bag(&bag);
    let mut snap = base_snapshot();
    snap.in_combat = true;
    set_seers_scene(&mut snap);
    snap.here = Some(TileInput {
        x: 2724,
        z: 3490,
        level: 0,
    });
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 1);
    assert!(
        iso.drain_interacts().is_empty(),
        "combat suppresses banking"
    );
    iso.join();
}

#[test]
fn loot_count_chicken_shape_observes_deposit_afterdeposit_and_return() {
    let mut iso = spawn(CHICKEN);
    let mut bag = serde_json::Map::new();
    bag.insert("bankStrategy".into(), serde_json::json!("Loot count"));
    bag.insert("bankEveryItems".into(), serde_json::json!(15));
    iso.post_settings_bag(&bag);

    let mut snap = base_snapshot();
    snap.here = Some(TileInput {
        x: 2724,
        z: 3490,
        level: 0,
    });
    snap.locs = std::slice::from_ref(seers_booth());
    snap.reach = seers_reach();
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 1);
    let open = one_action(&mut iso, "named booth opens through the native seat");
    assert!(matches!(
        &open.effect,
        HostEffect::Interaction(InteractReq::OpenBooth {
            x: 2725,
            z: 3490,
            level: 0,
            id: 2213,
            ..
        })
    ), "queued open is not completion; identity is the adjacent booth");
    accept_interaction(&mut iso, &open, 1);
    assert!(
        iso.probe("__after").is_err(),
        "afterDeposit must not run before a fresh snapshot"
    );

    let bones = [
        item_row("Bones", 526, 1, 0),
        item_row("Bronze arrow", 882, 50, 1),
    ];
    snap.tick = 2;
    snap.bank_open = true;
    snap.bank_loaded = true;
    snap.bank_generation = 1;
    snap.bank_side = &bones;
    snap.inv = &bones;
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 2);
    let deposit = one_action(&mut iso, "deposit operation reaches the host");
    assert!(matches!(
        deposit.effect,
        HostEffect::Interaction(request) if request == deposit_all(526, 0, 1)
    ));
    accept_interaction(&mut iso, &deposit, 2);
    assert!(
        iso.probe("__after").is_err(),
        "queued deposit is not completion"
    );

    let kept = [item_row("Bronze arrow", 882, 50, 1)];
    snap.tick = 3;
    snap.bank_side = &kept;
    snap.inv = &kept;
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 3);
    assert_eq!(
        iso.probe("__after").unwrap(),
        true,
        "afterDeposit runs after observed deposit"
    );
    assert!(
        iso.take_actions().is_empty(),
        "frozen bankNearest waits one tick after afterDeposit"
    );

    snap.tick = 4;
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 4);
    let action = one_action(&mut iso, "walk back starts next");
    assert!(
        matches!(
            &action.effect,
            HostEffect::Walk(request)
                if request.target == (api::snapshot::WorldTile { x: 3222, z: 3222, level: 0 })
                    && request.radius == 6
        ),
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
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 5);
    assert!(iso.take_actions().is_empty());
    assert_eq!(iso.probe("__log").unwrap(), "periodic bank: completed");
    assert_eq!(iso.probe("__status").unwrap(), "periodic bank run");
    iso.join();
}

#[test]
fn either_label_and_loot_count_label_both_trigger() {
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
    let mut iso = spawn(src);
    let mut snap = base_snapshot();
    snap.here = Some(TileInput {
        x: 2724,
        z: 3490,
        level: 0,
    });
    set_seers_scene(&mut snap);
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 1);
    assert_eq!(iso.probe("__shim").unwrap(), "items");
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
    let mut iso = spawn(src);
    let mut snap = base_snapshot();
    snap.here = Some(TileInput {
        x: 2724,
        z: 3490,
        level: 0,
    });
    set_seers_scene(&mut snap);
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 1);
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

/// Banking.ts:226–271 gives scene access precedence over a destination;
/// PeriodicBank.ts:46–53 forwards that destination without changing priority.
#[test]
fn supplied_destination_is_a_fallback_and_local_access_still_wins() {
    let src = r#"
import { PeriodicBank } from '../../api/tasks/PeriodicBank.js';
globalThis.__after = 0;
export default class T extends TaskBot {
    onStart() {
        this.add(new PeriodicBank({
            strategy: () => 'loot',
            itemsThreshold: () => 1,
            minutesThreshold: () => 10,
            countLoot: () => globalThis.__after ? 0 : 3,
            deposit: () => false,
            afterDeposit: () => { globalThis.__after++; },
            destination: () => ({ tile: { x: 2724, z: 3490, level: 0 } }),
            returnTo: () => null,
        }));
    }
}
"#;
    for local in [false, true] {
        let mut iso = spawn(src);
        let booth = if local {
            booth_at(3011, 3354)
        } else {
            *seers_booth()
        };
        let locs = [booth];
        let mut snap = base_snapshot();
        if local {
            snap.here = Some(TileInput {
                x: booth.x - 1,
                z: booth.z,
                level: 0,
            });
            snap.locs = &locs;
            snap.reach = open_reach_at(booth.x - 1, booth.z);
        }
        post_snapshot_input(&mut iso, &snap);
        tick(&mut iso, 1);
        let first = one_action(&mut iso, "local access or supplied destination");
        if local {
            assert!(matches!(
                &first.effect,
                HostEffect::Interaction(InteractReq::OpenBooth {
                    x: 3011,
                    z: 3354,
                    id: 2213,
                    ..
                })
            ), "local access must beat the distant destination");
            accept_interaction(&mut iso, &first, 1);
        } else {
            assert!(matches!(
                &first.effect,
                HostEffect::Walk(request)
                    if request.target == (api::snapshot::WorldTile {
                        x: 2724,
                        z: 3490,
                        level: 0,
                    }) && request.radius == 4
            ), "a supplied destination must bypass bank selection");
            arrive_walk(&mut iso, &first, 1);
            snap.tick = 2;
            snap.here = Some(TileInput {
                x: 2724,
                z: 3490,
                level: 0,
            });
            snap.locs = &locs;
            snap.reach = seers_reach();
            post_snapshot_input(&mut iso, &snap);
            tick(&mut iso, 2);
            let open = one_action(&mut iso, "adjacent booth opens after arrival");
            assert!(matches!(
                &open.effect,
                HostEffect::Interaction(InteractReq::OpenBooth {
                    x: 2725,
                    z: 3490,
                    id: 2213,
                    ..
                })
            ));
            accept_interaction(&mut iso, &open, 2);
        }
        assert_eq!(iso.probe("__after").unwrap(), 0);
        snap.tick = 3;
        snap.bank_open = true;
        snap.bank_generation = 1;
        post_snapshot_input(&mut iso, &snap);
        tick(&mut iso, 3);
        assert_eq!(
            iso.probe("__after").unwrap(),
            0,
            "open without loaded stock is not completion"
        );
        let side = [item_row("Coins", 995, 1, 0)];
        snap.bank_loaded = true;
        snap.bank_side = &side;
        for n in 4..=6 {
            snap.tick = n;
            post_snapshot_input(&mut iso, &snap);
            tick(&mut iso, n);
        }
        assert_eq!(iso.probe("__after").unwrap(), 1);
        iso.join();
    }
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
    let mut iso = spawn(src);
    iso.probe("globalThis.__junk = true").unwrap();
    let side = [
        item_row("Uncut sapphire", 1623, 1, 0),
        item_row("Coins", 995, 25, 1),
    ];
    let pack = [
        pack_row("Uncut sapphire", 1623, 1, 0),
        pack_row("Coins", 995, 25, 1),
    ];
    let mut snap = base_snapshot();
    snap.here = Some(TileInput {
        x: 2724,
        z: 3490,
        level: 0,
    });
    snap.bank_open = true;
    snap.bank_loaded = true;
    snap.bank_generation = 4;
    snap.bank_side = &side;
    snap.inv = &pack;
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 1);
    assert_eq!(iso.drain_interacts(), vec![deposit_all(1623, 0, 4)]);
    iso.join();

    let mut iso = spawn(src);
    iso.probe("globalThis.__junk = false").unwrap();
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 1);
    assert_eq!(iso.drain_interacts(), vec![deposit_all(995, 1, 4)]);
    iso.join();
}

#[test]
fn missing_access_backs_off_without_retry_spam() {
    let mut iso = spawn_with_facts(CHICKEN, picker_facts());
    let mut bag = serde_json::Map::new();
    bag.insert("bankStrategy".into(), serde_json::json!("Loot count"));
    iso.post_settings_bag(&bag);
    let mut snap = base_snapshot();
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 1);
    let pick = one_action(&mut iso, "no-scene banking must await the native picker");
    assert!(matches!(&pick.effect, HostEffect::BankPick(_)));
    no_bank_candidate(&mut iso, &pick, 2);
    for n in 2..=6u64 {
        snap.tick = n as i32;
        post_snapshot_input(&mut iso, &snap);
        tick(&mut iso, n);
    }
    assert!(
        iso.take_actions().is_empty(),
        "backoff must not retry-spam inside 180s"
    );
    iso.join();
}

#[test]
fn pause_and_session_reset_drop_late_callback_and_sends() {
    let mut iso = spawn(CHICKEN);
    let mut bag = serde_json::Map::new();
    bag.insert("bankStrategy".into(), serde_json::json!("Loot count"));
    iso.post_settings_bag(&bag);
    let mut snap = base_snapshot();
    snap.here = Some(TileInput {
        x: 2724,
        z: 3490,
        level: 0,
    });
    set_seers_scene(&mut snap);
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 1);
    let open = one_action(&mut iso, "nearby booth opens");
    assert!(matches!(
        &open.effect,
        HostEffect::Interaction(InteractReq::OpenBooth { .. })
    ));
    accept_interaction(&mut iso, &open, 1);

    iso.slot.pause();
    snap.tick = 2;
    snap.bank_open = true;
    snap.bank_loaded = true;
    snap.bank_generation = 9;
    let bones = [item_row("Bones", 526, 1, 0)];
    let pack = [pack_row("Bones", 526, 1, 0)];
    snap.bank_side = &bones;
    snap.inv = &pack;
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 2);
    assert!(
        iso.take_actions().is_empty(),
        "Pause/hold must not send deposit while frozen"
    );
    assert!(iso.probe("__after").is_err());

    iso.slot.resume();
    snap.tick = 3;
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 3);
    let deposit = one_action(&mut iso, "resumed bank deposit");
    assert!(matches!(
        deposit.effect,
        HostEffect::Interaction(InteractReq::InvButton { .. })
    ));

    iso.reset_session_work();
    snap.tick = 4;
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 4);
    assert!(
        iso.probe("__after").is_err(),
        "session replacement must not run the old afterDeposit"
    );
    assert!(
        iso.take_actions().iter().all(|action| !matches!(
            &action.effect,
            HostEffect::Interaction(InteractReq::Close)
        )),
        "old close must not land after reset"
    );
    iso.join();
}

#[test]
fn chicken_killer_closed_face_walks_approach_before_opening_the_same_booth() {
    let mut iso = spawn(CHICKEN);
    let mut bag = serde_json::Map::new();
    bag.insert("bankStrategy".into(), serde_json::json!("Loot count"));
    iso.post_settings_bag(&bag);
    let booth = booth_at(3011, 3354);
    let locs = [booth];
    let mut snap = base_snapshot();
    snap.here = Some(TileInput {
        x: 3010,
        z: 3355,
        level: 0,
    });
    snap.locs = &locs;
    snap.reach = diagonal_approach_reach();
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 1);
    let walk = one_action(&mut iso, "a diagonal approach walks to the same booth");
    assert!(matches!(
        &walk.effect,
        HostEffect::Walk(request)
            if request.target == (api::snapshot::WorldTile {
                x: 3011,
                z: 3355,
                level: 0,
            }) && request.radius == 0
    ), "the access must walk before it opens the booth");
    arrive_walk(&mut iso, &walk, 1);

    snap.tick = 2;
    snap.here = Some(TileInput {
        x: 3011,
        z: 3355,
        level: 0,
    });
    snap.reach = stand_ready_reach();
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 2);
    let open = one_action(&mut iso, "the selected booth opens after approach");
    assert!(matches!(
        &open.effect,
        HostEffect::Interaction(InteractReq::OpenBooth {
            x: 3011,
            z: 3354,
            level: 0,
            id: 2213,
            ..
        })
    ));
    iso.join();
}

#[test]
fn approach_dest_late_can_operate_opens_without_bank_generation_advance() {
    let mut iso = spawn(CHICKEN);
    let mut bag = serde_json::Map::new();
    bag.insert("bankStrategy".into(), serde_json::json!("Loot count"));
    iso.post_settings_bag(&bag);
    const GEN: u64 = 12;
    let booth = booth_at(3011, 3354);
    let locs = [booth];
    let mut snap = base_snapshot();
    snap.bank_generation = GEN;
    snap.here = Some(TileInput {
        x: 3010,
        z: 3355,
        level: 0,
    });
    snap.locs = &locs;
    snap.reach = diagonal_approach_reach();
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 1);
    let walk = one_action(&mut iso, "the native approach chooses a stand");
    assert!(matches!(
        &walk.effect,
        HostEffect::Walk(request)
            if request.target == (api::snapshot::WorldTile {
                x: 3011,
                z: 3355,
                level: 0,
            }) && request.radius == 0
    ));
    arrive_walk(&mut iso, &walk, 1);

    snap.tick = 2;
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 2);
    assert!(
        iso.take_actions().is_empty(),
        "must wait on native approach readiness, not bank generation"
    );

    snap.tick = 3;
    snap.here = Some(TileInput {
        x: 3011,
        z: 3355,
        level: 0,
    });
    snap.reach = stand_ready_reach();
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 3);
    let open = one_action(&mut iso, "a later reach view opens the booth");
    assert!(matches!(
        &open.effect,
        HostEffect::Interaction(InteractReq::OpenBooth {
            x: 3011,
            z: 3354,
            level: 0,
            id: 2213,
            ..
        })
    ));
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
    let mut iso = spawn(src);
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
        shape: 0,
        angle: 0,
    };
    let locs = [chest];
    let mut snap = base_snapshot();
    snap.here = Some(TileInput {
        x: 3308,
        z: 3120,
        level: 0,
    });
    snap.locs = &locs;
    snap.reach = open_reach_at(3308, 3120);
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 1);
    let open = one_action(&mut iso, "the selected chest receives the native open");
    assert!(matches!(
        &open.effect,
        HostEffect::Interaction(InteractReq::Loc {
            x: 3309,
            z: 3120,
            level: 0,
            action,
            id: Some(2693),
        }) if action == "Open"
    ));
    accept_interaction(&mut iso, &open, 1);

    let side = [item_row("Coins", 995, 25, 0)];
    let pack = [pack_row("Coins", 995, 25, 0)];
    snap.tick = 2;
    snap.bank_open = true;
    snap.bank_loaded = true;
    snap.bank_generation = 1;
    snap.bank_side = &side;
    snap.inv = &pack;
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 2);
    let deposit = one_action(&mut iso, "selected chest deposit");
    assert!(matches!(
        deposit.effect,
        HostEffect::Interaction(request) if request == deposit_all(995, 0, 1)
    ));
    iso.join();
}

/// Frozen calls `setStatus`, `log` and the deposit matcher synchronously:
/// never-settling status and log promises do not hold the run, and an async
/// matcher's Promise is truthy, so the first backpack row is deposited.
#[test]
fn synchronous_options_are_not_awaited() {
    let src = r#"
import { PeriodicBank } from '../../api/tasks/PeriodicBank.js';
export default class T extends TaskBot {
    onStart() {
        globalThis.__calls = [];
        this.add(new PeriodicBank({
            strategy: () => 'loot',
            itemsThreshold: () => 1,
            minutesThreshold: () => 10,
            countLoot: () => 2,
            deposit: async (name) => {
                globalThis.__calls.push(`deposit:${name}`);
                return name === 'Coins';
            },
            commonJunk: () => false,
            returnTo: () => null,
            setStatus: (status) => {
                globalThis.__calls.push(`status:${status}`);
                return new Promise(() => {});
            },
            log: (line) => {
                globalThis.__calls.push(`log:${line}`);
                return new Promise(() => {});
            },
        }));
    }
}
"#;
    let mut iso = spawn(src);
    let side = [item_row("Bones", 526, 1, 0), item_row("Coins", 995, 25, 1)];
    let pack = [pack_row("Bones", 526, 1, 0), pack_row("Coins", 995, 25, 1)];
    let mut snap = base_snapshot();
    snap.here = Some(TileInput {
        x: 2724,
        z: 3490,
        level: 0,
    });
    let locs = [*seers_booth()];
    snap.locs = &locs;
    snap.reach = open_reach_at(2724, 3490);
    snap.bank_open = true;
    snap.bank_loaded = true;
    snap.bank_generation = 4;
    snap.bank_side = &side;
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 1);
    assert_eq!(iso.drain_interacts(), vec![deposit_all(526, 0, 4)]);
    assert_eq!(
        iso.probe("__calls").unwrap(),
        serde_json::json!(["status:periodic bank run", "deposit:Bones"]),
        "status precedes the matcher, and neither returned promise holds the first deposit"
    );
    snap.tick = 2;
    snap.bank_open = false;
    snap.bank_loaded = false;
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 2);
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 3);
    assert_eq!(
        iso.probe("__calls").unwrap(),
        serde_json::json!([
            "status:periodic bank run",
            "deposit:Bones",
            "log:periodic bank: completed"
        ]),
        "the final log follows the deposit and its pending promise does not hold completion"
    );
    iso.join();
}

/// A booth beside the player with no usable native reach view is not opened:
/// the run fails closed and backs off with the frozen log line.
#[test]
fn an_adjacent_booth_without_an_approach_fact_fails_closed() {
    let mut iso = spawn(CHICKEN);
    let mut bag = serde_json::Map::new();
    bag.insert("bankStrategy".into(), serde_json::json!("Loot count"));
    iso.post_settings_bag(&bag);
    let mut snap = base_snapshot();
    snap.here = Some(TileInput {
        x: 2724,
        z: 3490,
        level: 0,
    });
    let locs = [*seers_booth()];
    snap.locs = &locs;
    post_snapshot_input(&mut iso, &snap);
    tick(&mut iso, 1);
    assert!(
        iso.drain_interacts().is_empty(),
        "no OpenBooth without native reach evidence"
    );
    assert_eq!(
        iso.probe("__log").unwrap(),
        "periodic bank: no bank reachable — will retry later"
    );
    iso.join();
}

#[test]
fn nearby_scene_booth_beats_a_distant_preset_unless_disabled() {
    for prefer in [true, false] {
        let source = format!(
            r#"
import {{ Banking }} from '../../api/bank/Banking.js';
globalThis.__ok = null;
export default class T extends LoopingBot {{
    async loop() {{
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = await Banking.open({{
            stand: {{x:3094,z:3492,level:0}}, preferNearby: {prefer}
        }});
    }}
}}"#
        );
        let mut iso = spawn(source);
        let mut local_locs = [*seers_booth()];
        let mut snap = base_snapshot();
        snap.here = Some(TileInput {
            x: 2724,
            z: 3490,
            level: 0,
        });
        snap.locs = &local_locs;
        snap.reach = open_reach_at(2724, 3490);
        post_snapshot_input(&mut iso, &snap);
        tick(&mut iso, 1);
        let action = one_action(&mut iso, "selecting the nearby booth or preset");
        if prefer {
            assert!(matches!(
                &action.effect,
                HostEffect::Interaction(InteractReq::OpenBooth {
                    x: 2725,
                    z: 3490,
                    id: 2213,
                    ..
                })
            ), "the reachable nearby booth must win");
            accept_interaction(&mut iso, &action, 1);
        } else {
            assert!(matches!(
                &action.effect,
                HostEffect::Walk(request)
                    if request.target == (api::snapshot::WorldTile {
                        x: 3094,
                        z: 3492,
                        level: 0,
                    }) && request.radius == 2
            ), "preferNearby:false must honor the preset");
            arrive_walk(&mut iso, &action, 1);
            snap.tick = 2;
            snap.here = Some(TileInput {
                x: 3094,
                z: 3492,
                level: 0,
            });
            local_locs = [booth_at(3094, 3493)];
            snap.locs = &local_locs;
            snap.reach = open_reach_at(3094, 3492);
            post_snapshot_input(&mut iso, &snap);
            tick(&mut iso, 2);
            let open = one_action(&mut iso, "the preset booth opens after arrival");
            assert!(matches!(
                &open.effect,
                HostEffect::Interaction(InteractReq::OpenBooth {
                    x: 3094,
                    z: 3493,
                    id: 2213,
                    ..
                })
            ));
            accept_interaction(&mut iso, &open, 2);
        }
        assert_eq!(iso.probe("__ok").unwrap(), serde_json::Value::Null);
        snap.tick = 3;
        snap.bank_open = true;
        snap.bank_generation = 1;
        post_snapshot_input(&mut iso, &snap);
        tick(&mut iso, 3);
        assert_eq!(iso.probe("__ok").unwrap(), serde_json::Value::Null);
        snap.tick = 4;
        snap.bank_loaded = true;
        post_snapshot_input(&mut iso, &snap);
        tick(&mut iso, 4);
        assert_eq!(iso.probe("__ok").unwrap(), true);
        iso.join();
    }
}
