// Task 38 — `Shop.open/buy/sell/close` composed end to end: the shim facade
// queues a caller's arguments, the Rust `script::shop` runtime owns the
// batching, the waits and the settlement, and the host re-resolves the exact
// posted row the queued op names. A queued click is never a transfer.

use api::identity::{ExecutionId, RequestId};
use api::script_contract::ChildOrigin;
use api::selected::RunKey;
use script::interact::InteractReq;
use script::native::{
    request::NativeKind,
    seat::{SeatEntry, SeatResult, SeatTerminal},
    seat_rows::{SeatLiveDetails, SeatPageInput, SeatResultValue, SeatStartArgs},
    snapshot_source::{
        ChatLineInput, ItemRowInput, NativeFactsInput, ReachViewInput, SceneEntityInput,
        SnapshotInput, TileInput,
    },
    NativePhase,
};
use script_js::wire::encode_snapshot_with_native;
use script_js::{LoadIsolate, LoadShape};
use serde_json::Value;
fn spawn_native(src: &str) -> LoadIsolate {
    LoadIsolate::spawn_with_execution(
        src.to_string(),
        LoadShape::CompatClass,
        vec![],
        ExecutionId::first(RunKey {
            slot: 37,
            run: 1,
            session: 1,
        }),
    )
    .unwrap()
}

fn post_native_seat_page(
    iso: &LoadIsolate,
    snap: &SnapshotInput<'_>,
    live: &[SeatEntry],
    results: &[SeatResult],
    details: &[SeatLiveDetails<'_>],
    values: &[SeatResultValue<'_>],
) {
    let page = SeatPageInput::new(live, results, details, 0)
        .expect("valid native seat page")
        .with_result_values(values)
        .expect("valid native seat result values");
    iso.post_snapshot(encode_snapshot_with_native(
        snap,
        NativeFactsInput {
            shop_player: Some(&[]),
            seat_page: Some(page),
            ..NativeFactsInput::default()
        },
    ));
    iso.on_game_tick(snap.tick);
    let _ = iso.probe("true").expect("posted seat page tick completes");
}
fn shop_live_entry(id: RequestId) -> SeatEntry {
    SeatEntry {
        id,
        parent: None,
        kind: NativeKind::Shop,
        origin: ChildOrigin::Started,
        depth: 1,
        decisions_consumed: 0,
        prompt: None,
    }
}

fn post_shop_live(iso: &LoadIsolate, snap: &SnapshotInput<'_>, id: RequestId) {
    let live = [shop_live_entry(id)];
    let details = [SeatLiveDetails {
        id,
        phase: NativePhase::Waiting,
        prompt: None,
    }];
    post_native_seat_page(iso, snap, &live, &[], &details, &[]);
}

fn post_shop_result(iso: &LoadIsolate, snap: &SnapshotInput<'_>, id: RequestId, bought: i32) {
    let results = [SeatResult {
        id,
        seq: 1,
        outcome: SeatTerminal::Done,
        prior_parent: None,
        session: Some(id),
        authority_revoked: false,
    }];
    let encoded = script::channel::encode(&Value::from(bought)).expect("encoded Shop result");
    let values = [SeatResultValue {
        id,
        value: &encoded,
    }];
    post_native_seat_page(iso, snap, &[], &results, &[], &values);
}

fn assert_shop_start(iso: &LoadIsolate, item_id: f64, quantity: f64) -> RequestId {
    let batches = iso.drain_seat_batches();
    let starts: Vec<_> = batches
        .iter()
        .flat_map(|batch| batch.batch.starts.iter())
        .collect();
    assert_eq!(starts.len(), 1, "one typed native Shop request starts");
    let start = starts[0];
    assert_eq!(start.kind, NativeKind::Shop);
    assert_eq!(
        start.args,
        SeatStartArgs::Shop(script::native_shop::ShopBuyByIdArgs { item_id, quantity }),
        "the requested object id and additional quantity cross the native seat wire"
    );
    assert!(
        iso.drain_interacts().is_empty(),
        "the shim does not queue a legacy ShopButton"
    );
    start.id
}

fn assert_no_shop_restart(iso: &LoadIsolate) {
    let batches = iso.drain_seat_batches();
    assert!(
        batches
            .iter()
            .flat_map(|batch| batch.batch.starts.iter())
            .all(|start| start.kind != NativeKind::Shop),
        "a live native Shop request is not started twice"
    );
    assert!(
        iso.drain_interacts().is_empty(),
        "the shim does not queue legacy interactions"
    );
}

#[path = "common/local_api_asset.rs"]
mod local_api_asset;
#[path = "common/local_game_data.rs"]
mod local_game_data;
fn data() -> std::sync::Arc<api::game_data::SelectedGameData> {
    api::game_data::for_revision(client::io::ClientRevision::R274).unwrap()
}

fn spawn(src: &str) -> LoadIsolate {
    LoadIsolate::spawn_with_game_data(src.to_string(), LoadShape::CompatClass, vec![], data())
        .unwrap()
}

/// One posted shop/stock/backpack row.
fn row<'a>(name: &'a str, id: i32, count: i32, component_id: i32, slot: i32) -> ItemRowInput<'a> {
    ItemRowInput {
        name: Some(name),
        count,
        id,
        ops: &[],
        noted: false,
        cert: -1,
        component_id,
        slot,
    }
}

fn npc<'a>(name: &'a str, actions: &'a [String]) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index: 0,
        id: 1,
        name: Some(name),
        x: 3222,
        z: 3222,
        level: 0,
        distance: 0,
        health: 100,
        max_health: 100,
        in_combat: false,
        animating: false,
        actions,
        reachable: true,
        reachable_adj: true,
        combat_level: 0,
        target_kind: 1,
        target_index: -1,
        size: 1,
        nx: 3222,
        nz: 3222,
        shape: 0,
        angle: 0,
    }
}

fn base<'a>() -> SnapshotInput<'a> {
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

/// Post a snapshot with `shop_player`: `None` = the shop side interface was
/// not decoded this rebuild, `Some(rows)` = decoded (possibly empty).
fn post(iso: &LoadIsolate, snap: &SnapshotInput<'_>, player: Option<&[ItemRowInput<'_>]>) {
    iso.post_snapshot(encode_snapshot_with_native(
        snap,
        NativeFactsInput {
            shop_player: player,
            ..Default::default()
        },
    ));
}

fn tick(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    let _ = iso.probe("true");
}

fn shop_button(
    kind: &str,
    name: &str,
    id: i32,
    slot: i32,
    component: i32,
    chunk: i32,
) -> InteractReq {
    InteractReq::ShopButton {
        kind: kind.into(),
        name: name.into(),
        id,
        slot,
        component,
        chunk,
    }
}

const BUY: &str = r#"
import { Shop } from '../../api/shop/Shop.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        try {
            globalThis.__ok = await Shop.buy(globalThis.__name || 'Lobster', globalThis.__qty ?? 1);
        } catch (e) {
            globalThis.__ok = String(e.message || e);
        }
    }
}
"#;

const BUY_BY_ID: &str = r#"
import { Shop } from '../../api/shop/Shop.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__bought = null;
        try {
            globalThis.__bought = await Shop.buyById(377, globalThis.__qty ?? 2);
        } catch (e) {
            globalThis.__bought = String(e.message || e);
        }
    }
}
"#;

const SELL: &str = r#"
import { Shop } from '../../api/shop/Shop.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        try {
            globalThis.__ok = await Shop.sell(globalThis.__name || 'Shark', globalThis.__qty ?? 1);
        } catch (e) {
            globalThis.__ok = String(e.message || e);
        }
    }
}
"#;

const OPEN: &str = r#"
import { Shop } from '../../api/shop/Shop.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        try {
            globalThis.__ok = await Shop.open(globalThis.__name || 'Shop keeper');
        } catch (e) {
            globalThis.__ok = String(e.message || e);
        }
    }
}
"#;

#[test]
fn buy_on_a_closed_shop_is_refused_without_a_packet() {
    if !crate::local_game_data::local_game_data_available(274) {
        return;
    }
    let iso = spawn(BUY);
    let snap = base();
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::from(0),
        "a closed shop buys nothing"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn buy_queues_the_frozen_batch_and_counts_only_the_moved_items() {
    if !crate::local_game_data::local_game_data_available(274) {
        return;
    }
    let iso = spawn(BUY);
    let stock = [row("Lobster", 377, 100, 9201, 5)];
    let held = [row("Lobster", 377, 0, 0, 0)];
    let first = [row("Lobster", 377, 10, 0, 0)];
    let all = [row("Lobster", 377, 25, 0, 0)];
    let mut snap = base();
    snap.shop_open = true;
    snap.shop_stock = &stock;
    snap.inv = &held;
    let _ = iso.probe("globalThis.__qty = 25");
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![
            shop_button("buy", "Lobster", 377, 5, 9201, 10),
            shop_button("buy", "Lobster", 377, 5, 9201, 10),
            shop_button("buy", "Lobster", 377, 5, 9201, 5),
        ],
        "25 buys as the frozen 10/5/1 decomposition, one tick's packets"
    );
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Null,
        "a queued batch is not a transfer"
    );

    // The first ten land: the batch settles one tick before it is counted.
    snap.tick = 2;
    snap.inv = &first;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 2);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Null,
        "settlement waits one tick before the recount"
    );

    // One tick later the ten count and the remaining fifteen are planned.
    snap.tick = 3;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 3);
    assert_eq!(
        iso.drain_interacts(),
        vec![
            shop_button("buy", "Lobster", 377, 5, 9201, 10),
            shop_button("buy", "Lobster", 377, 5, 9201, 5),
        ],
        "the remaining 15 batches as 10+5 against the same exact row"
    );

    snap.tick = 4;
    snap.inv = &all;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 4);
    snap.tick = 5;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 5);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::from(25),
        "the call reports the observed 25"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn sell_acts_on_the_shop_player_pack_not_the_stock_row_of_the_same_name() {
    if !crate::local_game_data::local_game_data_available(274) {
        return;
    }
    let iso = spawn(SELL);
    let stock = [row("Shark", 385, 50, 3900, 1)];
    let player = [row("Shark", 385, 4, 3823, 2)];
    let held = [row("Shark", 385, 9, 0, 0)];
    let sold = [row("Shark", 385, 5, 0, 0)];
    let mut snap = base();
    snap.shop_open = true;
    snap.shop_stock = &stock;
    snap.inv = &held;
    let _ = iso.probe("globalThis.__qty = 4");
    post(&iso, &snap, Some(&player));
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![
            shop_button("sell", "Shark", 385, 2, 3823, 1),
            shop_button("sell", "Shark", 385, 2, 3823, 1),
            shop_button("sell", "Shark", 385, 2, 3823, 1),
            shop_button("sell", "Shark", 385, 2, 3823, 1),
        ],
        "Sell resolves the shop's own player pack (3823), never the stock of the same name"
    );

    snap.tick = 2;
    snap.inv = &sold;
    post(&iso, &snap, Some(&player));
    tick(&iso, 2);
    snap.tick = 3;
    post(&iso, &snap, Some(&player));
    tick(&iso, 3);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::from(4),
        "the sold count is the observed inventory loss"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn sell_without_a_decoded_player_pack_fails_closed() {
    if !crate::local_game_data::local_game_data_available(274) {
        return;
    }
    let iso = spawn(SELL);
    let stock = [row("Shark", 385, 50, 3900, 1)];
    let mut snap = base();
    snap.shop_open = true;
    snap.shop_stock = &stock;
    // The side interface was not decoded this rebuild (`None`, not empty).
    post(&iso, &snap, None);
    tick(&iso, 1);
    let probe = iso.probe("__ok").unwrap();
    assert!(
        probe.as_str().unwrap_or("").contains("not impl"),
        "an unpublished player pack is an explicit failure, got {probe:?}"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

/// Frozen NatureCrafter un-note store: `Shop.sell(ESSENCE, n, i => i.id !==
/// ESSENCE_ID)` sells the noted stack, never the unnoted essence of the same
/// name, and asks `pick` again before every batch.
const SELL_PICK: &str = r#"
import { Shop } from '../../api/shop/Shop.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        globalThis.__asked = [];
        try {
            globalThis.__ok = await Shop.sell('Rune essence', globalThis.__qty, (i) => {
                globalThis.__asked.push(i);
                if (globalThis.__throw) throw new Error('pick broke');
                const picked = i.id !== 1436;
                return globalThis.__promise ? Promise.resolve(picked) : picked;
            });
        } catch (e) {
            globalThis.__ok = String(e.message || e);
        }
    }
}
"#;

#[test]
fn sell_with_pick_sells_the_row_pick_accepts_and_asks_again_per_batch() {
    if !crate::local_game_data::local_game_data_available(274) {
        return;
    }
    let iso = spawn(SELL_PICK);
    let player = [
        row("Rune essence", 1436, 1, 3823, 0),
        row("Rune essence", 1437, 60, 3823, 3),
    ];
    let held = [
        row("Rune essence", 1436, 1, 0, 0),
        row("Rune essence", 1437, 60, 0, 3),
    ];
    let first = [
        row("Rune essence", 1436, 1, 0, 0),
        row("Rune essence", 1437, 10, 0, 3),
    ];
    let rest = [
        row("Rune essence", 1436, 1, 0, 0),
        row("Rune essence", 1437, 5, 0, 3),
    ];
    let mut snap = base();
    snap.shop_open = true;
    snap.inv = &held;
    let _ = iso.probe("globalThis.__qty = 55");
    post(&iso, &snap, Some(&player));
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![shop_button("sell", "Rune essence", 1437, 3, 3823, 10); 5],
        "the batch sells the noted row pick accepted, in the caller's tick"
    );
    let asked = iso.probe("__asked").unwrap();
    assert_eq!(
        asked,
        serde_json::json!([
            { "id": 1436, "name": "Rune essence", "count": 1, "slot": 0, "comId": 3823 },
            { "id": 1437, "name": "Rune essence", "count": 60, "slot": 3, "comId": 3823 }
        ]),
        "pick sees each same-name row in order until one is accepted"
    );

    snap.tick = 2;
    snap.inv = &first;
    post(&iso, &snap, Some(&player));
    tick(&iso, 2);
    snap.tick = 3;
    post(&iso, &snap, Some(&player));
    tick(&iso, 3);
    assert_eq!(
        iso.drain_interacts(),
        vec![shop_button("sell", "Rune essence", 1437, 3, 3823, 5)],
        "the remaining five go from the row pick accepts again"
    );
    assert_eq!(
        iso.probe("__asked.length").unwrap(),
        4,
        "pick is asked again before the second batch"
    );

    snap.tick = 4;
    snap.inv = &rest;
    post(&iso, &snap, Some(&player));
    tick(&iso, 4);
    snap.tick = 5;
    post(&iso, &snap, Some(&player));
    tick(&iso, 5);
    assert_eq!(iso.probe("__ok").unwrap(), Value::from(55));
    iso.join();
}

#[test]
fn sell_treats_a_promise_returning_pick_as_sync_truthy() {
    if !crate::local_game_data::local_game_data_available(274) {
        return;
    }
    let iso = spawn(SELL_PICK);
    let player = [
        row("Rune essence", 1436, 2, 3823, 0),
        row("Rune essence", 1437, 2, 3823, 3),
    ];
    let held = [
        row("Rune essence", 1436, 2, 0, 0),
        row("Rune essence", 1437, 2, 0, 3),
    ];
    let sold = [
        row("Rune essence", 1436, 1, 0, 0),
        row("Rune essence", 1437, 2, 0, 3),
    ];
    let mut snap = base();
    snap.shop_open = true;
    snap.inv = &held;
    let _ = iso.probe("globalThis.__qty = 1; globalThis.__promise = true");
    post(&iso, &snap, Some(&player));
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![shop_button("sell", "Rune essence", 1436, 0, 3823, 1)],
        "frozen Array.find sees the Promise object as truthy without awaiting it"
    );
    assert_eq!(iso.probe("__asked.length").unwrap(), 1);

    snap.tick = 2;
    snap.inv = &sold;
    post(&iso, &snap, Some(&player));
    tick(&iso, 2);
    snap.tick = 3;
    post(&iso, &snap, Some(&player));
    tick(&iso, 3);
    assert_eq!(iso.probe("__ok").unwrap(), Value::from(1));
    iso.join();
}

#[test]
fn sell_with_a_refusing_or_throwing_pick_sends_nothing() {
    if !crate::local_game_data::local_game_data_available(274) {
        return;
    }
    let iso = spawn(SELL_PICK);
    let player = [row("Rune essence", 1436, 4, 3823, 0)];
    let mut snap = base();
    snap.shop_open = true;
    snap.inv = &player;
    let _ = iso.probe("globalThis.__qty = 4");
    post(&iso, &snap, Some(&player));
    tick(&iso, 1);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::from(0),
        "no accepted row sells nothing"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();

    let iso = spawn(SELL_PICK);
    let _ = iso.probe("globalThis.__qty = 4; globalThis.__throw = true");
    post(&iso, &snap, Some(&player));
    tick(&iso, 1);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        "pick broke",
        "what pick throws rejects the sale"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn open_presses_the_keepers_trade_op_and_reports_the_opened_shop() {
    if !crate::local_game_data::local_game_data_available(274) {
        return;
    }
    let iso = spawn(OPEN);
    let actions = ["Talk-to".to_string(), "Trade".to_string()];
    let npcs = [npc("Shop keeper", &actions)];
    let stock = [row("Lobster", 377, 100, 9201, 5)];
    let mut snap = base();
    snap.npcs = &npcs;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Npc {
            name: "Shop keeper".into(),
            action: "Trade".into(),
            index: Some(0),
        }],
        "open presses the posted keeper's Trade op, not an invented one"
    );
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Null,
        "the press is not the opened shop"
    );

    snap.tick = 2;
    snap.shop_open = true;
    snap.shop_stock = &stock;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 2);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(true));
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn open_without_a_keeper_resolves_false_without_a_packet() {
    if !crate::local_game_data::local_game_data_available(274) {
        return;
    }
    let iso = spawn(OPEN);
    let actions = ["Attack".to_string()];
    let guards = [npc("Guard", &actions)];
    let mut snap = base();
    snap.npcs = &guards;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Bool(false),
        "an absent keeper is false, like the frozen query"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

/// The Trade clicks in `ops`, whichever keeper row they name.
fn trades(ops: &[InteractReq]) -> usize {
    ops.iter()
        .filter(|op| {
            matches!(op, InteractReq::Npc { name, action, .. }
                if name == "Shop keeper" && action == "Trade")
        })
        .count()
}

/// The 274bot jive_kq_four race: the keeper opens the shop after the 0.2.1
/// 3000 ms window, and a second Trade then closed the shop it opened
/// (`OpNpcHandler.ts:47` -> `Player.ts:970-973`). One Trade must wait out
/// its tick deadline, however long the wall clock runs.
#[test]
fn open_sends_one_trade_while_the_keeper_takes_longer_than_three_seconds() {
    if !crate::local_game_data::local_game_data_available(274) {
        return;
    }
    let iso = spawn(OPEN);
    let actions = ["Talk-to".to_string(), "Trade".to_string()];
    let npcs = [npc("Shop keeper", &actions)];
    let stock = [row("Shantay pass", 1854, 100, 3900, 15)];
    let mut snap = base();
    snap.npcs = &npcs;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    assert_eq!(trades(&iso.drain_interacts()), 1);

    // The player stands next to the wandering keeper; no face is posted.
    std::thread::sleep(std::time::Duration::from_millis(3_100));
    for n in 2..=6 {
        snap.tick = n;
        post(&iso, &snap, Some(&[]));
        tick(&iso, n);
        assert_eq!(
            trades(&iso.drain_interacts()),
            0,
            "no second Trade on tick {n}"
        );
    }
    snap.tick = 7;
    snap.shop_open = true;
    snap.shop_stock = &stock;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 7);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(true));
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn open_ignores_a_usable_page_posted_on_its_trade_tick() {
    if !crate::local_game_data::local_game_data_available(274) {
        return;
    }
    let iso = spawn(OPEN);
    let actions = ["Trade".to_string()];
    let npcs = [npc("Shop keeper", &actions)];
    let stock = [row("Lobster", 377, 100, 9201, 5)];
    let mut snap = base();
    snap.npcs = &npcs;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    assert_eq!(trades(&iso.drain_interacts()), 1);
    // A page posted within the Trade's own tick belongs to an earlier
    // interaction the Trade is about to close.
    snap.shop_open = true;
    snap.shop_stock = &stock;
    post(&iso, &snap, Some(&[]));
    let _ = iso.probe("true");
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);
    snap.tick = 2;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 2);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(true));
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn cant_reach_ends_each_attempt_and_three_resolve_false() {
    if !crate::local_game_data::local_game_data_available(274) {
        return;
    }
    let iso = spawn(OPEN);
    let actions = ["Trade".to_string()];
    let npcs = [npc("Shop keeper", &actions)];
    let refusals: Vec<[ChatLineInput<'static>; 1]> = (1..=3)
        .map(|seq| {
            [ChatLineInput {
                seq,
                text: "I can't reach that!",
                type_: 0,
                username: None,
            }]
        })
        .collect();
    let mut snap = base();
    snap.npcs = &npcs;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    assert_eq!(trades(&iso.drain_interacts()), 1);
    for (n, lines) in (2..=4).zip(&refusals) {
        snap.tick = n;
        snap.chat_lines = &lines[..];
        post(&iso, &snap, Some(&[]));
        tick(&iso, n);
        let ops = iso.drain_interacts();
        if n < 4 {
            assert_eq!(trades(&ops), 1, "refusal on tick {n} retries");
            assert_eq!(iso.probe("__ok").unwrap(), Value::Null);
        } else {
            assert!(ops.is_empty(), "no fourth Trade");
            assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(false));
        }
    }
    iso.join();
}

/// No reach view is posted, so no route is measured: each attempt waits
/// the policy cap (`shop_open::OPEN_CAP_TICKS` = 116 ticks), not the
/// keeper's straight-line distance plus slack.
#[test]
fn three_expired_attempts_resolve_false_after_their_tick_deadlines() {
    const CAP: u64 = 116;
    if !crate::local_game_data::local_game_data_available(274) {
        return;
    }
    let iso = spawn(OPEN);
    let actions = ["Trade".to_string()];
    let npcs = [npc("Shop keeper", &actions)];
    let mut snap = base();
    snap.npcs = &npcs;
    let mut sent_on = Vec::new();
    let mut n = 1;
    loop {
        snap.tick = n;
        post(&iso, &snap, Some(&[]));
        tick(&iso, n);
        if trades(&iso.drain_interacts()) > 0 {
            sent_on.push(n);
        }
        if iso.probe("__ok").unwrap() != Value::Null {
            break;
        }
        n += 1;
        assert!(
            n < 4 * (CAP + 1),
            "Shop.open must end on its tick deadlines"
        );
    }
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(false));
    // The call fails one deadline after the third Trade without a fourth.
    let step = CAP + 1;
    assert_eq!(sent_on, vec![1, 1 + step, 1 + 2 * step]);
    assert_eq!(n, 1 + 3 * step);
    iso.join();
}

/// A posted flood that starts off the player's tile (stale: from before the
/// player last moved) measures nothing either: the attempt waits the cap.
#[test]
fn open_with_a_stale_flood_waits_the_cap() {
    const CAP: u64 = 116;
    if !crate::local_game_data::local_game_data_available(274) {
        return;
    }
    let iso = spawn(OPEN);
    let actions = ["Trade".to_string()];
    let npcs = [keeper_at(&actions, 16, 13)];
    // Flooded from x = 2; the player now stands on x = 3.
    let flood = corridor(2, 15, Some(16));
    let mut snap = base();
    snap.here = at(3);
    snap.npcs = &npcs;
    snap.reach = corridor_view(&flood);
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    assert_eq!(trades(&iso.drain_interacts()), 1);
    for n in 2..=1 + CAP {
        snap.tick = n;
        post(&iso, &snap, Some(&[]));
        tick(&iso, n);
        assert_eq!(trades(&iso.drain_interacts()), 0, "tick {n}");
    }
    snap.tick = 2 + CAP;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 2 + CAP);
    assert_eq!(trades(&iso.drain_interacts()), 1);
    iso.join();
}

/// An `if_close` (an NPC's hit, `npc_combat_melee.rs2:31,36`) shuts each
/// Trade's shop modal before its stock page is posted. `Shop.open` never
/// settles on it and never sends a shop button; three attempts end false.
#[test]
fn a_shop_closed_before_its_page_never_opens_and_ends_false() {
    if !crate::local_game_data::local_game_data_available(274) {
        return;
    }
    let iso = spawn(OPEN);
    let actions = ["Trade".to_string()];
    let npcs = [keeper_at(&actions, 12, 1)];
    let beside = corridor(11, 11, Some(12));
    let mut snap = base();
    snap.here = at(11);
    snap.npcs = &npcs;
    snap.reach = corridor_view(&beside);
    let mut sent_on = Vec::new();
    let mut n = 1;
    loop {
        snap.tick = n;
        // The modal shows the tick after each Trade, without its page,
        // and is closed again the tick after that.
        snap.shop_open = sent_on.last().is_some_and(|&t| n == t + 1);
        post(&iso, &snap, Some(&[]));
        tick(&iso, n);
        let ops = iso.drain_interacts();
        assert!(
            !ops.iter()
                .any(|op| matches!(op, InteractReq::ShopButton { .. })),
            "no shop button on tick {n}"
        );
        if trades(&ops) > 0 {
            sent_on.push(n);
        }
        if iso.probe("__ok").unwrap() != Value::Null {
            break;
        }
        n += 1;
        assert!(n < 100, "Shop.open must end on its tick deadlines");
    }
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(false));
    // Beside the keeper: route 0, so each attempt waits 0 + slack (12).
    assert_eq!(sent_on, vec![1, 14, 27]);
    assert_eq!(n, 40);
    iso.join();
}

const GRID: i32 = 24;

/// A posted reach view along row `z = 5` from `here_x` east to `to_x`:
/// each tile's dequeue rank is its distance from `here_x`, and the step
/// masks (`api::query::reach` `DIRS`: bit 0 west, bit 1 east) open each
/// step between neighbouring corridor tiles. `beside` puts the keeper on
/// blocked ground east of `to_x` with no wall between: the step onto the
/// keeper stays closed, the step from it back onto `to_x` is open, and that
/// is the engine's NPC reach. Without it, tiles east of `to_x` are not
/// reached (a wall or a closed door).
struct Corridor {
    walkable: Vec<u32>,
    reachable: Vec<u32>,
    reachable_adj: Vec<u32>,
    exact_rank: Vec<u16>,
    adjacent_rank: Vec<u16>,
    step: Vec<u8>,
}

fn corridor(here_x: i32, to_x: i32, beside: Option<i32>) -> Corridor {
    let n = (GRID * GRID) as usize;
    let index = |x: i32| (x * GRID + 5) as usize;
    let mut c = Corridor {
        walkable: vec![0; n.div_ceil(32)],
        reachable: vec![0; n.div_ceil(32)],
        reachable_adj: vec![0; n.div_ceil(32)],
        exact_rank: vec![u16::MAX; n],
        adjacent_rank: vec![u16::MAX; n],
        step: vec![0; n],
    };
    for x in here_x..=to_x {
        let i = index(x);
        let rank = u16::try_from(x - here_x).unwrap();
        c.walkable[i / 32] |= 1 << (i % 32);
        c.reachable[i / 32] |= 1 << (i % 32);
        c.reachable_adj[i / 32] |= 1 << (i % 32);
        c.exact_rank[i] = rank;
        c.adjacent_rank[i] = rank;
        if x > here_x {
            c.step[i] |= 1;
        }
        if x < to_x {
            c.step[i] |= 1 << 1;
        }
    }
    if let Some(x) = beside {
        // The keeper's tile, reached through its stand at `to_x`.
        let i = index(x);
        c.reachable_adj[i / 32] |= 1 << (i % 32);
        c.adjacent_rank[i] = u16::try_from(to_x - here_x).unwrap();
        c.step[i] |= 1;
    }
    c
}

fn corridor_view(c: &Corridor) -> ReachViewInput<'_> {
    ReachViewInput {
        available: true,
        base_x: 0,
        base_z: 0,
        level: 0,
        width: GRID,
        height: GRID,
        walkable: &c.walkable,
        reachable: &c.reachable,
        reachable_adj: &c.reachable_adj,
        exact_rank: &c.exact_rank,
        adjacent_rank: &c.adjacent_rank,
        step: &c.step,
        canlight: &[],
        stamp: 1,
    }
}

fn at(x: i32) -> Option<TileInput> {
    Some(TileInput { x, z: 5, level: 0 })
}

fn keeper_at<'a>(actions: &'a [String], x: i32, distance: i32) -> SceneEntityInput<'a> {
    SceneEntityInput {
        x,
        z: 5,
        distance,
        nx: x,
        nz: 5,
        ..npc("Shop keeper", actions)
    }
}

/// A keeper 13 walking steps away (inside the engine's 15-tile NPC view):
/// one Trade, no second one before the route deadline (Trade tick + 13 +
/// slack), and the shop that opens on that deadline tick is accepted.
#[test]
fn open_waits_out_the_keepers_route_with_one_trade() {
    if !crate::local_game_data::local_game_data_available(274) {
        return;
    }
    let iso = spawn(OPEN);
    let actions = ["Trade".to_string()];
    let npcs = [keeper_at(&actions, 16, 14)];
    let stock = [row("Air rune", 556, 100, 3900, 0)];
    let flood = corridor(2, 15, Some(16));
    let mut snap = base();
    snap.here = at(2);
    snap.npcs = &npcs;
    snap.reach = corridor_view(&flood);
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    assert_eq!(trades(&iso.drain_interacts()), 1);

    // Ticks without the shop: the slack the deadline adds is the gap the
    // no-flood attempts in `three_expired_attempts_…` show (more than 5).
    let mut n = 2;
    loop {
        snap.tick = n;
        post(&iso, &snap, Some(&[]));
        tick(&iso, n);
        let sent = trades(&iso.drain_interacts());
        if sent > 0 {
            break;
        }
        n += 1;
        assert!(n < 100, "the attempt must end on its deadline");
    }
    let deadline = n - 1;
    assert!(
        deadline >= 1 + 13 + 6,
        "the route length and slack both hold the retry back (retry on {n})"
    );
    iso.join();

    // The same walk, opening on the deadline tick: one Trade, then true.
    let iso = spawn(OPEN);
    let mut snap = base();
    snap.here = at(2);
    snap.npcs = &npcs;
    snap.reach = corridor_view(&flood);
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    assert_eq!(trades(&iso.drain_interacts()), 1);
    for n in 2..deadline {
        snap.tick = n;
        post(&iso, &snap, Some(&[]));
        tick(&iso, n);
        assert_eq!(trades(&iso.drain_interacts()), 0, "tick {n}");
    }
    snap.tick = deadline;
    snap.shop_open = true;
    snap.shop_stock = &stock;
    post(&iso, &snap, Some(&[]));
    tick(&iso, deadline);
    snap.tick = deadline + 1;
    post(&iso, &snap, Some(&[]));
    tick(&iso, deadline + 1);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(true));
    assert_eq!(trades(&iso.drain_interacts()), 0);
    iso.join();
}

/// A keeper behind a closed door: the flood reaches no stand, so nav walks
/// to it first; once beside it, one Trade.
#[test]
fn open_walks_to_a_door_blocked_keeper_then_trades_once() {
    if !crate::local_game_data::local_game_data_available(274) {
        return;
    }
    let iso = spawn(OPEN);
    let actions = ["Trade".to_string()];
    let npcs = [keeper_at(&actions, 12, 7)];
    let stock = [row("Air rune", 556, 100, 3900, 0)];
    let shut = corridor(5, 9, None);
    let mut snap = base();
    snap.here = at(5);
    snap.npcs = &npcs;
    snap.reach = corridor_view(&shut);
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    let ops = iso.drain_interacts();
    assert_eq!(trades(&ops), 0, "no Trade to an unreachable keeper");
    assert!(
        matches!(
            ops.as_slice(),
            [InteractReq::WalkNear {
                x: 12,
                z: 5,
                radius: 1,
                ..
            }]
        ),
        "nav walks to the keeper first, got {ops:?}"
    );

    // Nav opened the door and arrived beside the keeper.
    let open = corridor(11, 11, Some(12));
    snap.tick = 2;
    snap.here = at(11);
    snap.npcs = &npcs;
    snap.reach = corridor_view(&open);
    post(&iso, &snap, Some(&[]));
    tick(&iso, 2);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Npc {
            name: "Shop keeper".into(),
            action: "Trade".into(),
            index: Some(0),
        }]
    );
    snap.tick = 3;
    snap.shop_open = true;
    snap.shop_stock = &stock;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 3);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(true));
    iso.join();
}

/// A nav walk that ends without reaching the keeper ends `Shop.open` false
/// without a Trade (here: the user takes the walk over).
#[test]
fn open_fails_without_a_trade_when_the_walk_to_the_keeper_fails() {
    if !crate::local_game_data::local_game_data_available(274) {
        return;
    }
    let iso = spawn(OPEN);
    let actions = ["Trade".to_string()];
    let npcs = [keeper_at(&actions, 12, 7)];
    let shut = corridor(5, 9, None);
    let mut snap = base();
    snap.here = at(5);
    snap.npcs = &npcs;
    snap.reach = corridor_view(&shut);
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    assert_eq!(trades(&iso.drain_interacts()), 0);
    snap.tick = 2;
    snap.user_move_intent_seq = 1;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 2);
    snap.tick = 3;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 3);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(false));
    assert_eq!(trades(&iso.drain_interacts()), 0);
    iso.join();
}

#[test]
fn closing_the_shop_mid_batch_ends_the_transfer_with_its_partial_count() {
    if !crate::local_game_data::local_game_data_available(274) {
        return;
    }
    let iso = spawn(BUY);
    let stock = [row("Lobster", 377, 100, 9201, 5)];
    let held = [row("Lobster", 377, 0, 0, 0)];
    let first = [row("Lobster", 377, 10, 0, 0)];
    let mut snap = base();
    snap.shop_open = true;
    snap.shop_stock = &stock;
    snap.inv = &held;
    let _ = iso.probe("globalThis.__qty = 25");
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts().len(), 3);

    // Ten landed, then the shop went down: the transfer stops on the posted
    // close with the partial progress it observed.
    snap.tick = 2;
    snap.shop_open = false;
    snap.shop_stock = &[];
    snap.inv = &first;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 2);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::from(0),
        "a shop that went down ends the transfer where it stood"
    );
    snap.tick = 3;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 3);
    assert!(
        iso.drain_interacts().is_empty(),
        "a closed shop receives no further shop-button"
    );
    iso.join();
}

/// An `if_close` (an NPC's hit) after one batch settled: `Shop.buy` ends
/// with the settled count, and the closed shop gets no further button.
#[test]
fn an_if_close_after_a_settled_batch_ends_buy_with_its_settled_count() {
    if !crate::local_game_data::local_game_data_available(274) {
        return;
    }
    let iso = spawn(BUY);
    let stock = [row("Lobster", 377, 100, 9201, 5)];
    let held = [row("Lobster", 377, 0, 0, 0)];
    let first = [row("Lobster", 377, 10, 0, 0)];
    let mut snap = base();
    snap.shop_open = true;
    snap.shop_stock = &stock;
    snap.inv = &held;
    let _ = iso.probe("globalThis.__qty = 25");
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts().len(), 3);
    // Ten land and count; the remaining fifteen go out on tick 3.
    snap.tick = 2;
    snap.inv = &first;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 2);
    snap.tick = 3;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 3);
    assert_eq!(iso.drain_interacts().len(), 2);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);

    // The hit closes the shop before the second batch lands.
    snap.tick = 4;
    snap.shop_open = false;
    snap.shop_stock = &[];
    post(&iso, &snap, Some(&[]));
    tick(&iso, 4);
    assert_eq!(iso.probe("__ok").unwrap(), Value::from(10));
    snap.tick = 5;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 5);
    assert!(
        iso.drain_interacts().is_empty(),
        "a closed shop receives no further shop-button"
    );
    iso.join();
}

#[test]
fn pause_hold_and_session_reset_abort_a_parked_transfer() {
    if !crate::local_game_data::local_game_data_available(274) {
        return;
    }
    let iso = spawn(BUY);
    let stock = [row("Lobster", 377, 100, 9201, 5)];
    let held = [row("Lobster", 377, 0, 0, 0)];
    let moved = [row("Lobster", 377, 10, 0, 0)];
    let mut snap = base();
    snap.shop_open = true;
    snap.shop_stock = &stock;
    snap.inv = &held;
    let _ = iso.probe("globalThis.__qty = 25");
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts().len(), 3);

    // Pause freezes the outstanding batch: the moved counts do not settle it.
    iso.pause();
    snap.tick = 2;
    snap.inv = &moved;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 2);
    tick(&iso, 3);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Null,
        "Pause must not complete the parked transfer"
    );
    iso.resume();

    // Guardian hold freezes it too.
    snap.hold = true;
    snap.tick = 4;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 4);
    tick(&iso, 5);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Null,
        "Guardian hold must freeze the parked settlement"
    );

    // A session abort drops the operation without a late packet.
    iso.reset_session_work(None);
    snap.hold = false;
    snap.tick = 6;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 6);
    tick(&iso, 7);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::from(0),
        "a reset session returns no count"
    );
    assert!(
        iso.drain_interacts().is_empty(),
        "a reset session must not click again"
    );
    iso.join();
}

/// §28.7 row 12: the actual Shop.sell promise retains the 12 already credited
/// before a kept reconnect. A closed-shop keyframe cannot credit two more
/// missing items or issue a post-relog shop button.
#[test]
fn sell_kept_reconnect_preserves_partial_count_without_a_post_relog_click() {
    if !crate::local_game_data::local_game_data_available(274) {
        return;
    }
    for post_relog_count in [8, 6] {
        let iso = spawn(SELL);
        let stock = [row("Shark", 385, 50, 3900, 1)];
        let player = [row("Shark", 385, 20, 3823, 2)];
        let held = [row("Shark", 385, 20, 0, 0)];
        let credited = [row("Shark", 385, 8, 0, 0)];
        let after_relog = [row("Shark", 385, post_relog_count, 0, 0)];
        let mut snap = base();
        snap.shop_open = true;
        snap.shop_stock = &stock;
        snap.inv = &held;
        iso.probe("globalThis.__qty = 20").unwrap();
        post(&iso, &snap, Some(&player));
        tick(&iso, 1);
        assert_eq!(iso.drain_interacts().len(), 2, "two 10-item Sell buttons");

        snap.tick = 2;
        snap.inv = &credited;
        post(&iso, &snap, Some(&player));
        tick(&iso, 2);
        assert_eq!(iso.probe("__ok").unwrap(), Value::Null);

        iso.reconnect_session_work(None);
        assert!(
            iso.drain_interacts().is_empty(),
            "an old-session queued Sell must not reach the new session"
        );
        snap.tick = 3;
        snap.hold = true;
        snap.shop_open = false;
        snap.shop_stock = &[];
        snap.inv = &after_relog;
        post(&iso, &snap, Some(&[]));
        tick(&iso, 3);
        assert_eq!(iso.probe("__ok").unwrap(), Value::Null);
        assert!(iso.drain_interacts().is_empty());

        snap.hold = false;
        snap.tick = 4;
        post(&iso, &snap, Some(&[]));
        tick(&iso, 4);
        snap.tick = 5;
        post(&iso, &snap, Some(&[]));
        tick(&iso, 5);
        assert_eq!(
            iso.probe("__ok").unwrap(),
            Value::from(12),
            "the closed-shop keyframe cannot credit another backpack loss"
        );
        assert!(
            iso.drain_interacts().is_empty(),
            "the kept sale must not click the closed shop after relog"
        );
        iso.join();
    }
}

const INDEX_D_TS: &str = include_str!("../compat-js/index.d.ts");
const MODULES_JSON: &str = include_str!("../compat-js/modules.json");
const ABI_CONTRACT: &str = include_str!("../compat-js/abi-contract.json");

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_shop__unitprice__contract() {
    if !api::test_support::game_data_available() {
        return;
    }
    let src = r#"
import { unitPrice } from '../../api/shop/StockModel.js';
export default class T extends LoopingBot {
    loop() {
        const item = { cost: 2, baseline: 500 };
        const shop = { sell: 1300, delta: 20 };
        globalThis.__prices = [
            unitPrice(item, shop, 20),
            unitPrice(item, shop, 600),
            unitPrice(item, shop, 400),
            unitPrice({ cost: 0, baseline: 0 }, { sell: 100, delta: 10 }, 0),
        ];
    }
}
"#;
    let iso = spawn(src);
    let snap = base();
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    assert_eq!(
        iso.probe("__prices").unwrap(),
        serde_json::json!([12, 1, 6, 1]),
        "unitPrice applies the posted stock haggle, floor, and one-gp minimum"
    );
    iso.join();
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_shop__buyoutcost__contract() {
    if !api::test_support::game_data_available() {
        return;
    }
    let src = r#"
import { buyoutCost } from '../../api/shop/shopPrice.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__costs = [
            buyoutCost(20, 3),
            buyoutCost(20, 2.5),
            buyoutCost(20, 0),
        ];
    }
}
"#;
    let iso = spawn(src);
    let snap = base();
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    assert_eq!(
        iso.probe("__costs").unwrap(),
        serde_json::json!([6, 6, 0]),
        "buyoutCost sums each stock-dependent unit and follows the loop boundary"
    );
    iso.join();
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_shop__shopbuyprice__contract() {
    if !api::test_support::game_data_available() {
        return;
    }
    let src = r#"
import { shopBuyPrice } from '../../api/shop/shopPrice.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__prices = [
            shopBuyPrice(20, 0),
            shopBuyPrice(20, 1),
            shopBuyPrice(20, 10),
            shopBuyPrice(20, 700),
        ];
    }
}
"#;
    let iso = spawn(src);
    let snap = base();
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    assert_eq!(
        iso.probe("__prices").unwrap(),
        serde_json::json!([2, 2, 4, 102]),
        "shopBuyPrice prices the next unit at each stock count with bounded haggle"
    );
    iso.join();
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_shop__shop_buybyid__contract() {
    let iso = spawn_native(BUY_BY_ID);
    let stock = [
        row("Lobster", 999, 100, 3900, 3),
        row("Lobster", 377, 1, 3900, 1),
        row("Lobster", 377, 8, 3900, 2),
    ];
    let coins = [row("Coins", 995, 1000, 0, 0)];
    let mut snap = base();
    snap.shop_open = true;
    snap.shop_stock = &stock;
    snap.inv = &coins;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    let id = assert_shop_start(&iso, 377.0, 2.0);
    assert_eq!(iso.probe("__bought").unwrap(), Value::Null);

    snap.tick = 2;
    post_shop_live(&iso, &snap, id);
    assert_eq!(iso.probe("__bought").unwrap(), Value::Null);
    assert_no_shop_restart(&iso);

    snap.tick = 3;
    post_shop_result(&iso, &snap, id, 1);
    assert_eq!(
        iso.probe("__bought").unwrap(),
        Value::from(1),
        "buyById resolves with the actual count only after the native terminal result"
    );
    assert_no_shop_restart(&iso);
    iso.join();

    let zero = spawn_native(BUY_BY_ID);
    zero.probe("globalThis.__qty = 0").unwrap();
    let mut snap = base();
    post(&zero, &snap, Some(&[]));
    tick(&zero, 1);
    let id = assert_shop_start(&zero, 377.0, 0.0);
    assert_eq!(zero.probe("__bought").unwrap(), Value::Null);
    snap.tick = 2;
    post_shop_result(&zero, &snap, id, 0);
    assert_eq!(
        zero.probe("__bought").unwrap(),
        Value::from(0),
        "zero quantity also waits for the native terminal result"
    );
    assert_no_shop_restart(&zero);
    zero.join();

    let fractional = spawn_native(BUY_BY_ID);
    fractional.probe("globalThis.__qty = 1.5").unwrap();
    let mut snap = base();
    post(&fractional, &snap, Some(&[]));
    tick(&fractional, 1);
    let id = assert_shop_start(&fractional, 377.0, 1.5);
    snap.tick = 2;
    post_shop_result(&fractional, &snap, id, 2);
    assert_eq!(
        fractional.probe("__bought").unwrap(),
        Value::from(2),
        "a positive fractional quantity resolves with every completed unit"
    );
    fractional.join();
}

#[test]
fn buy_by_id_pause_and_reset_do_not_repeat_or_credit_unobserved_items() {
    let iso = spawn_native(BUY_BY_ID);
    let stock = [row("Lobster", 377, 100, 3900, 1)];
    let coins = [row("Coins", 995, 1000, 0, 0)];
    let mut snap = base();
    snap.shop_open = true;
    snap.shop_stock = &stock;
    snap.inv = &coins;
    iso.probe("globalThis.__qty = 3").unwrap();
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    let id = assert_shop_start(&iso, 377.0, 3.0);
    assert_eq!(iso.probe("__bought").unwrap(), Value::Null);

    iso.pause();
    snap.tick = 2;
    post_shop_live(&iso, &snap, id);
    snap.tick = 3;
    post_shop_live(&iso, &snap, id);
    assert_eq!(iso.probe("__bought").unwrap(), Value::Null);
    assert_no_shop_restart(&iso);

    iso.resume();
    snap.hold = true;
    snap.tick = 4;
    post_shop_live(&iso, &snap, id);
    snap.tick = 5;
    post_shop_live(&iso, &snap, id);
    assert_eq!(iso.probe("__bought").unwrap(), Value::Null);
    assert_no_shop_restart(&iso);

    iso.reset_session_work(None);
    snap.hold = false;
    snap.tick = 6;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 6);
    tick(&iso, 7);
    assert_eq!(
        iso.probe("__bought").unwrap(),
        Value::from(0),
        "a reset session aborts the native Shop request without a fabricated count"
    );
    assert_no_shop_restart(&iso);
    iso.join();
}

#[test]
fn buy_by_id_kept_reconnect_returns_settled_count_without_a_post_relog_click() {
    let iso = spawn_native(BUY_BY_ID);
    let stock = [row("Lobster", 377, 100, 3900, 1)];
    let coins = [row("Coins", 995, 1000, 0, 0)];
    let mut snap = base();
    snap.shop_open = true;
    snap.shop_stock = &stock;
    snap.inv = &coins;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    let id = assert_shop_start(&iso, 377.0, 2.0);

    snap.tick = 2;
    post_shop_live(&iso, &snap, id);
    assert_eq!(iso.probe("__bought").unwrap(), Value::Null);
    iso.reconnect_session_work(None);

    snap.tick = 3;
    snap.hold = true;
    snap.shop_open = false;
    snap.shop_stock = &[];
    post_shop_live(&iso, &snap, id);
    assert_eq!(iso.probe("__bought").unwrap(), Value::Null);
    assert_no_shop_restart(&iso);

    snap.tick = 4;
    snap.hold = false;
    post_shop_result(&iso, &snap, id, 2);
    assert_eq!(
        iso.probe("__bought").unwrap(),
        Value::from(2),
        "kept reconnect returns the observed native result"
    );
    assert_no_shop_restart(&iso);
    iso.join();
}

#[test]
#[expect(non_snake_case, reason = "the census names this removal test")]
fn compat_v1_shop__shop_player__removed() {
    if !api::test_support::game_data_available() {
        return;
    }
    let src = r#"
import { Shop } from '../../api/shop/Shop.js';
const trap = key => {
    try { Shop[key]; return 'read'; }
    catch (error) { return String(error.message || error); }
};
globalThis.__shop_player = {
    own: Object.prototype.hasOwnProperty.call(Shop, 'player'),
    descriptor: Object.getOwnPropertyDescriptor(Shop, 'player') === undefined,
    listed: Object.keys(Shop).includes('player'),
    removed: trap('player'),
    unknown: trap('__noSuchShopMember'),
    siblings: [
        Shop.isOpen, Shop.stock, Shop.open, Shop.buy, Shop.buyById,
        Shop.sell, Shop.sellAll, Shop.close,
    ].every(member => typeof member === 'function'),
};
export default class T extends LoopingBot {
    loop() {}
}
"#;
    let iso = spawn(src);
    let snap = base();
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    let facts = iso.probe("__shop_player").unwrap();
    assert_eq!(facts["own"], false);
    assert_eq!(facts["descriptor"], true);
    assert_eq!(facts["listed"], false);
    assert_eq!(facts["siblings"], true);
    assert_eq!(
        facts["removed"]
            .as_str()
            .map(|message| message.replace("player", "<key>")),
        facts["unknown"]
            .as_str()
            .map(|message| message.replace("__noSuchShopMember", "<key>")),
        "Shop.player uses the generic unknown-member trap"
    );

    let start = INDEX_D_TS
        .find("declare module '*api/shop/Shop.js' {")
        .expect("index.d.ts declares the Shop module");
    let block = &INDEX_D_TS[start..];
    let block = &block[..block.find("\n}\n").expect("the Shop module block closes")];
    assert!(
        !block.contains("player()"),
        "index.d.ts still promises Shop.player"
    );
    assert!(
        !ABI_CONTRACT.contains("Shop.player"),
        "abi-contract.json still promises Shop.player"
    );
    assert!(
        MODULES_JSON.contains("api/shop/Shop.js"),
        "the supported Shop module remains registered"
    );
    iso.join();
}
