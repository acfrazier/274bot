// Task 38 — `Shop.open/buy/sell/close` composed end to end: the shim facade
// queues a caller's arguments, the Rust `script::shop` runtime owns the
// batching, the waits and the settlement, and the host re-resolves the exact
// posted row the queued op names. A queued click is never a transfer.

use script::isolate_fb::{
    encode_snapshot_with_native, ItemRowInput, NativeFactsInput, ReachViewInput, SceneEntityInput,
    SnapshotInput, TileInput,
};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};
use serde_json::Value;

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
        attacked_by_player: false,
        widgets: &[],
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

#[test]
fn a_pick_callback_is_not_mapped_and_has_no_packet() {
    let src = r#"
import { Shop } from '../../api/shop/Shop.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        try {
            globalThis.__ok = await Shop.sell('Shark', 1, (i) => i.id !== 385);
        } catch (e) {
            globalThis.__ok = String(e.message || e);
        }
    }
}
"#;
    let iso = spawn(src);
    let player = [row("Shark", 385, 4, 3823, 2)];
    let mut snap = base();
    snap.shop_open = true;
    post(&iso, &snap, Some(&player));
    tick(&iso, 1);
    let probe = iso.probe("__ok").unwrap();
    assert!(
        probe.as_str().unwrap_or("").contains("not impl"),
        "the caller-row selector stays an explicit failure, got {probe:?}"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn open_presses_the_keepers_trade_op_and_reports_the_opened_shop() {
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
            index: None,
        }],
        "open presses the posted Trade op, not an invented one"
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

#[test]
fn closing_the_shop_mid_batch_ends_the_transfer_with_its_partial_count() {
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

#[test]
fn pause_hold_and_session_reset_abort_a_parked_transfer() {
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
    iso.reset_session_work();
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
