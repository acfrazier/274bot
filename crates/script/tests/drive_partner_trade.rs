// driveActivePartnerTrade: one frozen drivePartnerTrade.ts iteration per
// call. The caller's callbacks run in the frozen order with the frozen
// labels and log lines; Rust reads the screens and sends the trade ops.

use script::isolate_fb::{ItemRowInput, ReachViewInput, SnapshotInput, TileInput};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};
use std::thread::sleep;
use std::time::Duration;

const TRADE_CLOSE_DEBOUNCE_MS: u64 = 600;

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

const CARD: &str = r#"
import { driveActivePartnerTrade } from '../../api/trade/drivePartnerTrade.js';
import { Trade } from '../../api/trade/Trade.js';
export default class T extends LoopingBot {
    async loop() {
        if (!Trade.active() || (globalThis.__left ?? 1) <= 0) return;
        globalThis.__left = (globalThis.__left ?? 1) - 1;
        const ev = (globalThis.__ev = globalThis.__ev || []);
        const flax = () => (globalThis.__rs2b0t_host.snapshot.inv || [])
            .filter(r => r.name === 'Flax').reduce((n, r) => n + r.count, 0);
        const opts = {
            role: globalThis.__role || 'receiver',
            partners: ['partner'],
            theirProductMatch: n => { ev.push('match:' + n); return n.toLowerCase().includes('flax'); },
            productNamesToOffer: () => { ev.push('names'); return globalThis.__names ?? ['Flax']; },
            setStatus: s => { ev.push('status:' + s); },
            log: m => { ev.push('log:' + m); },
            inventoryMetric: () => { ev.push('metric'); return globalThis.__metric ? globalThis.__metric() : flax(); },
            onMissingPartner: () => { ev.push('missing'); return globalThis.__missing || 'wait'; },
            receiverCanAccept: n => { ev.push('gate:' + n); return globalThis.__gate ?? true; },
            myOfferReady: () => { ev.push('ready'); return globalThis.__ready ? globalThis.__ready() : Trade.myOffer().length > 0; },
            onDecline: r => { ev.push('decline:' + r); },
            onComplete: d => { ev.push('complete:' + d); },
            verifyGiverPartner: true,
            labels: globalThis.__labels ?? {
                confirming: 'confirming trade',
                waitHeader: 'reading trade partner',
                waitOffer: 'waiting for runner to offer flax',
                accepting: 'accepting flax from runner',
                offering: 'offering flax to spinner',
                acceptingOffer: 'accepting trade offer',
                declining: 'declining trade',
            },
        };
        for (const k of globalThis.__drop || []) delete opts[k];
        try {
            await driveActivePartnerTrade(opts);
            ev.push('end');
        } catch (e) {
            ev.push('threw:' + (e && e.message));
        }
    }
}
"#;

fn events(iso: &LoadIsolate) -> Vec<String> {
    serde_json::from_value(iso.probe("globalThis.__ev || []").unwrap()).unwrap()
}

/// Take the events since the last call.
fn take(iso: &LoadIsolate) -> Vec<String> {
    // In place: a running iteration holds the array.
    serde_json::from_value(iso.probe("(globalThis.__ev || []).splice(0)").unwrap()).unwrap()
}

fn set(iso: &LoadIsolate, js: &str) {
    iso.probe(js).unwrap();
}

fn offer_screen<'a>(partner: Option<&'a str>) -> SnapshotInput<'a> {
    let mut snap = base();
    snap.trade_offer_open = true;
    snap.trade_partner = partner;
    snap.trade_accept_id = 3420;
    snap.trade_decline_id = 3422;
    snap
}

fn flax_rows(n: usize) -> Vec<ItemRowInput<'static>> {
    (0..n)
        .map(|i| row("Flax", 1779, 1, 3416, i as i32, false))
        .collect()
}

fn strs(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn matches(n: usize) -> Vec<String> {
    vec!["match:Flax".to_string(); n]
}

#[test]
fn inactive_trade_calls_nothing_and_sends_nothing() {
    let iso = spawn(CARD);
    post(&iso, &base());
    tick(&iso, 1);
    assert!(events(&iso).is_empty());
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn receiver_counts_twice_gates_then_labels_and_accepts_in_the_same_tick() {
    // FlaxRunner's 28 unstackable flax rows: 2 x 28 matches, the gate,
    // the status, the metric and the log all run before Accept, in tick 1.
    let iso = spawn(CARD);
    let theirs = flax_rows(28);
    let mut snap = offer_screen(Some("partner"));
    snap.trade_theirs = &theirs;
    post(&iso, &snap);
    tick(&iso, 1);
    let mut want = matches(56);
    want.extend(strs(&[
        "gate:28",
        "status:accepting flax from runner",
        "metric",
        "log:trade: clicking Accept on the offer screen (offer)",
    ]));
    assert_eq!(take(&iso), want, "frozen receiver order");
    assert_eq!(iso.drain_interacts(), vec![if_button(3420)]);
    snap.tick = 2;
    snap.trade_offer_open = false;
    snap.trade_confirm_open = true;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(
        take(&iso),
        strs(&["log:trade: offer accepted — confirm screen is up", "end"])
    );
    iso.join();
}

#[test]
fn missing_header_asks_the_caller_before_the_status_and_can_decline() {
    let iso = spawn(CARD);
    let mut snap = offer_screen(None);
    post(&iso, &snap);
    tick(&iso, 1);
    tick(&iso, 2);
    assert_eq!(
        take(&iso),
        strs(&["missing", "status:reading trade partner", "end"])
    );
    assert!(iso.drain_interacts().is_empty());

    set(
        &iso,
        "globalThis.__missing = 'decline'; globalThis.__left = 1",
    );
    tick(&iso, 3);
    assert_eq!(
        take(&iso),
        strs(&[
            "missing",
            "status:declining trade",
            "log:trade: declining — partner name never appeared on the modal",
        ])
    );
    assert_eq!(iso.drain_interacts(), vec![if_button(3422)]);
    snap.tick = 4;
    snap.trade_offer_open = false;
    post(&iso, &snap);
    tick(&iso, 4);
    assert_eq!(
        take(&iso),
        strs(&[
            "log:trade: decline clicked — screen now closed",
            "decline:partner header timeout",
            "end",
        ])
    );
    iso.join();
}

#[test]
fn a_refused_gate_declines_with_its_reason_and_never_shows_accepting() {
    let iso = spawn(CARD);
    set(&iso, "globalThis.__gate = { ok: false, reason: 'no room' }");
    let theirs = flax_rows(2);
    let mut snap = offer_screen(Some("Partner"));
    snap.trade_theirs = &theirs;
    post(&iso, &snap);
    tick(&iso, 1);
    let mut want = matches(4);
    want.extend(strs(&[
        "gate:2",
        "status:declining trade",
        "log:trade: declining (no room)",
    ]));
    assert_eq!(take(&iso), want);
    assert_eq!(iso.drain_interacts(), vec![if_button(3422)]);
    snap.tick = 2;
    snap.trade_offer_open = false;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(
        take(&iso),
        strs(&[
            "log:trade: decline clicked — screen now closed",
            "decline:no room",
            "end",
        ])
    );
    iso.join();
}

#[test]
fn a_stranger_is_declined_with_the_frozen_reason() {
    let iso = spawn(CARD);
    let theirs = flax_rows(1);
    let mut snap = offer_screen(Some("stranger"));
    snap.trade_theirs = &theirs;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        take(&iso),
        strs(&[
            "match:Flax",
            "status:declining trade",
            "log:trade: declining (not a configured partner (stranger))",
        ])
    );
    iso.join();
}

#[test]
fn giver_offers_then_accepts_on_the_next_iteration() {
    let iso = spawn(CARD);
    set(&iso, "globalThis.__role = 'giver'");
    let side = [row("Flax", 1779, 24, 3322, 0, false)];
    let mut snap = offer_screen(Some("partner"));
    snap.trade_side = &side;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        take(&iso),
        strs(&["ready", "names", "status:offering flax to spinner"])
    );
    assert_eq!(iso.drain_interacts(), vec![inv_button(1779, 0, 3322, 4)]);
    let mine = [row("Flax", 1779, 24, 3415, 0, false)];
    snap.tick = 2;
    snap.trade_mine = &mine;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(take(&iso), strs(&["ready", "end"]), "the offer registered");

    set(&iso, "globalThis.__left = 1");
    tick(&iso, 3);
    assert_eq!(
        take(&iso),
        strs(&[
            "ready",
            "metric",
            "status:accepting trade offer",
            "log:trade: clicking Accept on the offer screen (offer)",
        ]),
        "the giver reads the metric before the acceptingOffer status"
    );
    assert_eq!(iso.drain_interacts(), vec![if_button(3420)]);
    iso.join();
}

#[test]
fn an_async_callback_result_is_a_value_not_awaited() {
    // Frozen `myOfferReady?.()` is not awaited: a promise is truthy.
    let iso = spawn(CARD);
    set(
        &iso,
        "globalThis.__role = 'giver'; globalThis.__ready = () => Promise.resolve(false)",
    );
    let mut snap = offer_screen(Some("partner"));
    snap.trade_side = &[];
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        take(&iso),
        strs(&[
            "ready",
            "metric",
            "status:accepting trade offer",
            "log:trade: clicking Accept on the offer screen (offer)",
        ])
    );
    iso.join();
}

#[test]
fn nothing_to_offer_uses_the_frozen_unlabelled_status() {
    let iso = spawn(CARD);
    set(&iso, "globalThis.__role = 'giver'; globalThis.__names = []");
    post(&iso, &offer_screen(Some("partner")));
    tick(&iso, 1);
    assert_eq!(
        take(&iso),
        strs(&[
            "ready",
            "names",
            "status:mule: nothing to offer — declining",
            "log:trade: declining (nothing to offer)",
        ])
    );
    assert_eq!(iso.drain_interacts(), vec![if_button(3422)]);
    iso.join();
}

fn confirm(iso: &LoadIsolate, inv_after: &[ItemRowInput<'_>]) -> Vec<String> {
    let inv = [row("Flax", 1779, 24, 0, 0, false)];
    let mut snap = base();
    snap.trade_confirm_open = true;
    snap.trade_partner = Some("partner");
    snap.trade_accept_id = 3546;
    snap.inv = &inv;
    post(iso, &snap);
    tick(iso, 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(3546)]);
    snap.tick = 2;
    snap.trade_confirm_open = false;
    snap.inv = inv_after;
    post(iso, &snap);
    tick(iso, 2);
    sleep(Duration::from_millis(TRADE_CLOSE_DEBOUNCE_MS + 100));
    for n in 3..=4 {
        snap.tick = n;
        post(iso, &snap);
        tick(iso, n);
    }
    take(iso)
}

#[test]
fn confirm_reports_the_metric_delta_after_a_stable_close_and_a_beat() {
    let iso = spawn(CARD);
    assert_eq!(
        confirm(&iso, &[]),
        strs(&[
            "status:confirming trade",
            "metric",
            "log:trade: clicking Accept on the confirm screen (confirm)",
            "log:trade: confirm wait satisfied after the last click — screen now closed",
            "metric",
            "complete:-24",
            "end",
        ])
    );
    iso.join();
}

#[test]
fn defaults_are_the_frozen_mule_labels_and_log_line() {
    let iso = spawn(CARD);
    set(
        &iso,
        "globalThis.__labels = {}; globalThis.__drop = ['onComplete']",
    );
    let events = confirm(&iso, &[]);
    assert_eq!(events[0], "status:mule: confirming trade");
    assert_eq!(
        events[events.len() - 2],
        "log:mule: trade complete (inv Δ-24)"
    );
    iso.join();
}

#[test]
fn an_undefined_metric_is_nan_as_in_frozen_arithmetic() {
    let iso = spawn(CARD);
    set(&iso, "globalThis.__metric = () => undefined");
    let events = confirm(&iso, &[]);
    assert_eq!(events[events.len() - 2], "complete:NaN");
    iso.join();
}

#[test]
fn reset_ends_the_iteration_without_later_callbacks_or_sends() {
    let iso = spawn(CARD);
    let theirs = flax_rows(1);
    let mut snap = offer_screen(Some("partner"));
    snap.trade_theirs = &theirs;
    post(&iso, &snap);
    tick(&iso, 1);
    take(&iso);
    iso.drain_interacts();
    iso.reset_session_work();
    snap.tick = 2;
    snap.trade_offer_open = false;
    snap.trade_confirm_open = true;
    post(&iso, &snap);
    tick(&iso, 2);
    tick(&iso, 3);
    assert_eq!(take(&iso), strs(&["end"]), "no confirm log after a reset");
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}
