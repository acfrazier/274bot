// Task 156 — already-open Flax partner exchange: JS projects caller
// callbacks/metrics, Rust owns posted identity, offer-then-confirm
// sequencing, settlement and cancellation. A raw close is not a transfer.

use script::isolate_fb::{ItemRowInput, ReachViewInput, SnapshotInput, TileInput};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};
use serde_json::Value;

fn spawn(src: &str) -> LoadIsolate {
    LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn row<'a>(
    name: &'a str,
    id: i32,
    count: i32,
    component_id: i32,
    slot: i32,
    noted: bool,
) -> ItemRowInput<'a> {
    ItemRowInput {
        name: Some(name),
        count,
        id,
        ops: &[],
        noted,
        cert: -1,
        component_id,
        slot,
    }
}

fn base<'a>() -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: Some(TileInput {
            x: 2719,
            z: 3471,
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

fn post(iso: &LoadIsolate, snap: &SnapshotInput<'_>) {
    iso.post_snapshot(script::isolate_fb::encode_snapshot(snap));
}

fn tick(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    let _ = iso.probe("true");
}

fn inv_button(id: i32, slot: i32, component: i32, operation: i32) -> InteractReq {
    InteractReq::InvButton {
        id,
        slot,
        component,
        operation,
        bank_generation: 0,
    }
}

fn if_button(component_id: i32) -> InteractReq {
    InteractReq::IfButton { component_id }
}

const GIVER: &str = r#"
import { driveActivePartnerTrade } from '../../api/trade/drivePartnerTrade.js';
import { Trade } from '../../api/trade/Trade.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__calls = [];
        globalThis.__complete = null;
        globalThis.__decline = null;
        globalThis.__ok = null;
        try {
            await driveActivePartnerTrade({
                role: 'giver',
                partners: [globalThis.__partner || 'spinner'],
                theirProductMatch: () => false,
                productNamesToOffer: () => {
                    globalThis.__calls.push('productNamesToOffer');
                    return globalThis.__names ?? ['Flax'];
                },
                setStatus: s => { globalThis.__status = s; },
                log: m => { globalThis.__log = m; },
                inventoryMetric: () => {
                    globalThis.__calls.push('inventoryMetric');
                    const inv = (globalThis.__rs2b0t_host.snapshot.inv || []);
                    return inv
                        .filter(r => (r.name || '').toLowerCase() === 'flax')
                        .reduce((n, r) => n + (r.count || 0), 0);
                },
                verifyGiverPartner: true,
                myOfferReady: () => {
                    globalThis.__calls.push('myOfferReady');
                    return (Trade.myOffer() || []).some(r => (r.name || '').toLowerCase() === 'flax');
                },
                onMissingPartner: () => {
                    globalThis.__calls.push('onMissingPartner');
                    return globalThis.__missing || 'wait';
                },
                onDecline: reason => {
                    globalThis.__calls.push('onDecline');
                    globalThis.__decline = reason;
                },
                onComplete: delta => {
                    globalThis.__calls.push('onComplete');
                    globalThis.__complete = delta;
                },
                labels: { offering: 'offering flax to spinner', confirming: 'confirming trade with spinner' },
            });
            globalThis.__ok = 'done';
        } catch (e) {
            globalThis.__ok = String(e.message || e);
        }
    }
}
"#;

const RECEIVER: &str = r#"
import { driveActivePartnerTrade } from '../../api/trade/drivePartnerTrade.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__calls = [];
        globalThis.__complete = null;
        globalThis.__decline = null;
        globalThis.__ok = null;
        try {
            await driveActivePartnerTrade({
                role: 'receiver',
                partners: [globalThis.__partner || 'runner'],
                theirProductMatch: n => {
                    globalThis.__calls.push('theirProductMatch');
                    return (n || '').toLowerCase().includes('flax');
                },
                productNamesToOffer: () => [],
                setStatus: s => { globalThis.__status = s; },
                log: () => {},
                inventoryMetric: () => {
                    globalThis.__calls.push('inventoryMetric');
                    const inv = (globalThis.__rs2b0t_host.snapshot.inv || []);
                    return inv
                        .filter(r => (r.name || '').toLowerCase() === 'flax')
                        .reduce((n, r) => n + (r.count || 0), 0);
                },
                onMissingPartner: () => 'wait',
                receiverCanAccept: count => {
                    globalThis.__calls.push('receiverCanAccept');
                    globalThis.__gate = count;
                    if (globalThis.__refuse) return { ok: false, reason: 'no room' };
                    return true;
                },
                onDecline: reason => {
                    globalThis.__calls.push('onDecline');
                    globalThis.__decline = reason;
                },
                onComplete: delta => {
                    globalThis.__calls.push('onComplete');
                    globalThis.__complete = delta;
                },
            });
            globalThis.__ok = 'done';
        } catch (e) {
            globalThis.__ok = String(e.message || e);
        }
    }
}
"#;

fn flax_side() -> [ItemRowInput<'static>; 1] {
    [row("Flax", 1779, 24, 3322, 0, false)]
}

fn flax_mine() -> [ItemRowInput<'static>; 1] {
    [row("Flax", 1779, 24, 3415, 0, false)]
}

fn flax_inv() -> [ItemRowInput<'static>; 1] {
    [row("Flax", 1779, 24, 0, 0, false)]
}

fn calls_contain(iso: &LoadIsolate, name: &str) -> bool {
    iso.probe("__calls")
        .ok()
        .and_then(|v| v.as_array().cloned())
        .map(|rows| rows.iter().any(|row| row.as_str() == Some(name)))
        .unwrap_or(false)
}

#[test]
fn inactive_trade_returns_without_not_impl_or_callbacks() {
    let iso = spawn(GIVER);
    post(&iso, &base());
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), "done");
    assert_eq!(iso.probe("__complete").unwrap(), Value::Null);
    assert_eq!(iso.probe("__decline").unwrap(), Value::Null);
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn giver_offer_then_confirm_completes_only_after_metric_change() {
    let iso = spawn(GIVER);
    let side = flax_side();
    let inv = flax_inv();
    let mine = flax_mine();
    let mut snap = base();
    snap.trade_offer_open = true;
    snap.trade_partner = Some("spinner");
    snap.trade_side = &side;
    snap.trade_accept_id = 3420;
    snap.trade_decline_id = 3422;
    snap.inv = &inv;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![inv_button(1779, 0, 3322, 4)],
        "giver must Offer-All unnoted flax via native inv-button 4"
    );
    assert!(calls_contain(&iso, "productNamesToOffer"));
    assert!(calls_contain(&iso, "inventoryMetric"));

    snap.tick = 2;
    snap.trade_mine = &mine;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.drain_interacts(), vec![if_button(3420)]);
    assert!(calls_contain(&iso, "myOfferReady"));

    snap.tick = 3;
    snap.trade_offer_open = false;
    snap.trade_confirm_open = true;
    snap.trade_accept_id = 3546;
    post(&iso, &snap);
    tick(&iso, 3);
    assert_eq!(iso.drain_interacts(), vec![if_button(3546)]);

    snap.tick = 4;
    snap.trade_confirm_open = false;
    snap.trade_mine = &[];
    snap.inv = &[];
    post(&iso, &snap);
    tick(&iso, 4);
    assert_eq!(iso.probe("__ok").unwrap(), "done");
    assert_eq!(iso.probe("__complete").unwrap(), Value::from(-24));
    assert_eq!(iso.probe("__decline").unwrap(), Value::Null);
    assert!(calls_contain(&iso, "onComplete"));
    assert!(!calls_contain(&iso, "onDecline"));
    iso.join();
}

#[test]
fn receiver_matched_product_completes_on_inventory_gain() {
    let iso = spawn(RECEIVER);
    let theirs = flax_mine();
    let inv = flax_inv();
    let mut snap = base();
    snap.trade_offer_open = true;
    snap.trade_partner = Some("runner");
    snap.trade_theirs = &theirs;
    snap.trade_accept_id = 3420;
    snap.trade_decline_id = 3422;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(3420)]);
    assert!(calls_contain(&iso, "theirProductMatch"));
    assert!(calls_contain(&iso, "receiverCanAccept"));
    assert_eq!(iso.probe("__gate").unwrap(), Value::from(24));

    snap.tick = 2;
    snap.trade_offer_open = false;
    snap.trade_confirm_open = true;
    snap.trade_accept_id = 3546;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.drain_interacts(), vec![if_button(3546)]);

    snap.tick = 3;
    snap.trade_confirm_open = false;
    snap.trade_theirs = &[];
    snap.inv = &inv;
    post(&iso, &snap);
    tick(&iso, 3);
    assert_eq!(iso.probe("__ok").unwrap(), "done");
    assert_eq!(iso.probe("__complete").unwrap(), Value::from(24));
    assert_eq!(iso.probe("__decline").unwrap(), Value::Null);
    iso.join();
}

#[test]
fn wrong_counterpart_declines_without_complete() {
    let iso = spawn(GIVER);
    let side = flax_side();
    let inv = flax_inv();
    let mut snap = base();
    snap.trade_offer_open = true;
    snap.trade_partner = Some("stranger");
    snap.trade_side = &side;
    snap.trade_accept_id = 3420;
    snap.trade_decline_id = 3422;
    snap.inv = &inv;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(3422)]);

    snap.tick = 2;
    snap.trade_offer_open = false;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.probe("__ok").unwrap(), "done");
    assert_eq!(iso.probe("__complete").unwrap(), Value::Null);
    assert_eq!(iso.probe("__decline").unwrap(), "stranger");
    assert!(calls_contain(&iso, "onDecline"));
    iso.join();
}

#[test]
fn missing_header_invokes_the_caller_hook_and_can_decline() {
    let iso = spawn(GIVER);
    let _ = iso.probe("globalThis.__missing = 'decline'");
    let side = flax_side();
    let inv = flax_inv();
    let mut snap = base();
    snap.trade_offer_open = true;
    snap.trade_side = &side;
    snap.trade_accept_id = 3420;
    snap.trade_decline_id = 3422;
    snap.inv = &inv;
    post(&iso, &snap);
    tick(&iso, 1);
    assert!(calls_contain(&iso, "onMissingPartner"));
    assert_eq!(iso.drain_interacts(), vec![if_button(3422)]);

    snap.tick = 2;
    snap.trade_offer_open = false;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.probe("__decline").unwrap(), "partner header timeout");
    assert_eq!(iso.probe("__complete").unwrap(), Value::Null);
    iso.join();
}

#[test]
fn stale_partner_change_does_not_complete() {
    let iso = spawn(GIVER);
    let side = flax_side();
    let inv = flax_inv();
    let mine = flax_mine();
    let mut snap = base();
    snap.trade_offer_open = true;
    snap.trade_partner = Some("spinner");
    snap.trade_side = &side;
    snap.trade_accept_id = 3420;
    snap.trade_decline_id = 3422;
    snap.inv = &inv;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts().len(), 1);

    snap.tick = 2;
    snap.trade_partner = Some("other");
    snap.trade_mine = &mine;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.drain_interacts(), vec![if_button(3422)]);

    snap.tick = 3;
    snap.trade_offer_open = false;
    post(&iso, &snap);
    tick(&iso, 3);
    assert_eq!(iso.probe("__complete").unwrap(), Value::Null);
    assert_eq!(iso.probe("__decline").unwrap(), "stale-phase");
    iso.join();
}

#[test]
fn no_progress_close_is_not_successful_completion() {
    let iso = spawn(GIVER);
    let side = flax_side();
    let inv = flax_inv();
    let mine = flax_mine();
    let mut snap = base();
    snap.trade_offer_open = true;
    snap.trade_partner = Some("spinner");
    snap.trade_side = &side;
    snap.trade_accept_id = 3420;
    snap.trade_decline_id = 3422;
    snap.inv = &inv;
    post(&iso, &snap);
    tick(&iso, 1);
    let _ = iso.drain_interacts();

    snap.tick = 2;
    snap.trade_mine = &mine;
    post(&iso, &snap);
    tick(&iso, 2);
    let _ = iso.drain_interacts();

    snap.tick = 3;
    snap.trade_offer_open = false;
    snap.trade_confirm_open = true;
    snap.trade_accept_id = 3546;
    post(&iso, &snap);
    tick(&iso, 3);
    let _ = iso.drain_interacts();

    snap.tick = 4;
    snap.trade_confirm_open = false;
    post(&iso, &snap);
    tick(&iso, 4);
    assert_eq!(iso.probe("__ok").unwrap(), "done");
    assert_eq!(iso.probe("__complete").unwrap(), Value::Null);
    assert_eq!(iso.probe("__decline").unwrap(), "no-progress");
    iso.join();
}

#[test]
fn empty_names_and_noted_side_refuse_without_offer_sends() {
    let iso = spawn(GIVER);
    let _ = iso.probe("globalThis.__names = []");
    let inv = flax_inv();
    let mut snap = base();
    snap.trade_offer_open = true;
    snap.trade_partner = Some("spinner");
    snap.trade_accept_id = 3420;
    snap.trade_decline_id = 3422;
    snap.inv = &inv;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(3422)]);
    snap.tick = 2;
    snap.trade_offer_open = false;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.probe("__decline").unwrap(), "nothing to offer");
    iso.join();

    let iso = spawn(GIVER);
    let noted = [row("Flax", 1780, 24, 3322, 0, true)];
    let mut noted_snap = base();
    noted_snap.trade_offer_open = true;
    noted_snap.trade_partner = Some("spinner");
    noted_snap.trade_side = &noted;
    noted_snap.trade_accept_id = 3420;
    noted_snap.trade_decline_id = 3422;
    noted_snap.inv = &inv;
    post(&iso, &noted_snap);
    tick(&iso, 1);
    assert!(
        iso.drain_interacts()
            .iter()
            .all(|op| !matches!(op, InteractReq::InvButton { .. })),
        "noted flax must not send Offer-All"
    );
    noted_snap.tick = 2;
    noted_snap.trade_offer_open = false;
    post(&iso, &noted_snap);
    tick(&iso, 2);
    assert_eq!(iso.probe("__complete").unwrap(), Value::Null);
    assert_eq!(iso.probe("__decline").unwrap(), "offerAll failed");
    iso.join();
}

#[test]
fn receiver_unintended_own_offer_declines() {
    let iso = spawn(RECEIVER);
    let mine = [row("Coins", 995, 1, 3415, 0, false)];
    let theirs = flax_mine();
    let mut snap = base();
    snap.trade_offer_open = true;
    snap.trade_partner = Some("runner");
    snap.trade_mine = &mine;
    snap.trade_theirs = &theirs;
    snap.trade_accept_id = 3420;
    snap.trade_decline_id = 3422;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(3422)]);
    snap.tick = 2;
    snap.trade_offer_open = false;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.probe("__decline").unwrap(), "unintended own offer");
    assert_eq!(iso.probe("__complete").unwrap(), Value::Null);
    iso.join();
}

#[test]
fn pause_hold_and_reset_do_not_send_late_actions_or_callbacks() {
    let iso = spawn(GIVER);
    let side = flax_side();
    let inv = flax_inv();
    let mine = flax_mine();
    let mut snap = base();
    snap.trade_offer_open = true;
    snap.trade_partner = Some("spinner");
    snap.trade_side = &side;
    snap.trade_accept_id = 3420;
    snap.trade_decline_id = 3422;
    snap.inv = &inv;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts().len(), 1);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);

    iso.pause();
    snap.tick = 2;
    snap.trade_mine = &mine;
    post(&iso, &snap);
    tick(&iso, 2);
    tick(&iso, 3);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);
    assert!(iso.drain_interacts().is_empty());
    iso.resume();

    snap.hold = true;
    snap.tick = 4;
    post(&iso, &snap);
    tick(&iso, 4);
    tick(&iso, 5);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);
    assert!(iso.drain_interacts().is_empty());

    iso.reset_session_work();
    snap.hold = false;
    snap.tick = 6;
    post(&iso, &snap);
    tick(&iso, 6);
    tick(&iso, 7);
    assert_eq!(iso.probe("__ok").unwrap(), "done");
    assert_eq!(iso.probe("__complete").unwrap(), Value::Null);
    assert_eq!(iso.probe("__decline").unwrap(), Value::Null);
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}
