// Task 126 — Trade request/offer/remove/accept/decline composed end to end:
// JS marshals arguments and the pick projection, Rust owns identity, the
// count-dialog wait and settlement, and a queued click is not a transfer.

use script::isolate_fb::{
    ItemRowInput, ReachViewInput, SceneEntityInput, SnapshotInput, TileInput,
};
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

fn player<'a>(name: &'a str, distance: i32, actions: &'a [String]) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index: 3,
        id: 3,
        name: Some(name),
        x: 3222,
        z: 3222,
        level: 0,
        distance,
        health: 99,
        max_health: 99,
        in_combat: false,
        animating: false,
        actions,
        reachable: true,
        reachable_adj: true,
        combat_level: 1,
        target_kind: 0,
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

fn post(iso: &LoadIsolate, snap: &SnapshotInput<'_>) {
    iso.post_snapshot(script::isolate_fb::encode_snapshot(snap));
}

fn tick(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    let _ = iso.probe("true");
}

const REQUEST: &str = r#"
import { Trade } from '../../api/trade/Trade.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        try {
            globalThis.__ok = await Trade.request(globalThis.__name || 'bob');
        } catch (e) {
            globalThis.__ok = String(e.message || e);
        }
    }
}
"#;

const OFFER_ALL: &str = r#"
import { Trade } from '../../api/trade/Trade.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        try {
            globalThis.__ok = await Trade.offerAll('Rune essence', (i) => i.id === 1436);
        } catch (e) {
            globalThis.__ok = String(e.message || e);
        }
    }
}
"#;

const OFFER_ALL_NAME: &str = r#"
import { Trade } from '../../api/trade/Trade.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = await Trade.offerAll('Rune essence');
    }
}
"#;

const OFFER: &str = r#"
import { Trade } from '../../api/trade/Trade.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        try {
            globalThis.__ok = await Trade.offer('Rune essence', globalThis.__n ?? 10, (i) => i.id === 1436);
        } catch (e) {
            globalThis.__ok = String(e.message || e);
        }
    }
}
"#;

const REMOVE: &str = r#"
import { Trade } from '../../api/trade/Trade.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        globalThis.__ok = await Trade.removeAll();
    }
}
"#;

const ACCEPT: &str = r#"
import { Trade } from '../../api/trade/Trade.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = await Trade.accept();
    }
}
"#;

const DECLINE: &str = r#"
import { Trade } from '../../api/trade/Trade.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = 'ran';
        await Trade.decline();
        globalThis.__ok = 'done';
    }
}
"#;

fn trade_ops() -> [String; 4] {
    [
        "Attack".into(),
        "Follow".into(),
        "Report".into(),
        "Trade with".into(),
    ]
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

#[test]
fn request_presses_the_posted_trade_with_op_not_a_short_trade_label() {
    let iso = spawn(REQUEST);
    let actions = trade_ops();
    let players = [player("bob", 2, &actions)];
    let mut snap = base();
    snap.players = &players;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Player {
            name: "bob".into(),
            action: "Trade with".into(),
        }],
        "request must emit the posted op-4 label, not Trade"
    );
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(true));
    iso.join();
}

#[test]
fn request_without_a_player_or_trade_op_is_false_without_a_packet() {
    let iso = spawn(REQUEST);
    let actions = ["Attack".to_string(), "Follow".to_string()];
    let players = [player("bob", 1, &actions)];
    let mut snap = base();
    snap.players = &players;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(false));
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn offer_all_sends_the_unnoted_id_and_slot_not_the_noted_stack() {
    let iso = spawn(OFFER_ALL);
    let side = [
        row("Rune essence", 1437, 27, 3322, 0, true),
        row("Rune essence", 1436, 25, 3322, 1, false),
    ];
    let mut snap = base();
    snap.trade_offer_open = true;
    snap.trade_partner = Some("bob");
    snap.trade_side = &side;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![inv_button(1436, 1, 3322, 4)],
        "pick must keep unnoted 1436 / slot 1; Offer All is inv-button 4"
    );
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(true));
    iso.join();
}

#[test]
fn offer_all_refuses_a_noted_selection_and_a_missing_screen() {
    let iso = spawn(OFFER_ALL);
    let noted_only = [row("Rune essence", 1437, 27, 3322, 0, true)];
    let mut snap = base();
    snap.trade_offer_open = true;
    snap.trade_partner = Some("bob");
    snap.trade_side = &noted_only;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(false));
    assert!(
        iso.drain_interacts().is_empty(),
        "a noted identity must not send"
    );
    iso.join();

    let iso = spawn(OFFER_ALL_NAME);
    let side = [row("Rune essence", 1436, 25, 3322, 1, false)];
    let mut closed = base();
    closed.trade_side = &side;
    post(&iso, &closed);
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(false));
    assert!(
        iso.drain_interacts().is_empty(),
        "a closed offer screen must not send"
    );
    iso.join();
}

#[test]
fn offer_all_refuses_a_wrong_id_or_slot_projection() {
    let src = r#"
export default class T extends LoopingBot {
    loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        const fn = globalThis.rustyscript.functions.__rs2b0t_trade;
        const begin = fn({ op: 'begin', kind: 'offerAll', name: 'Rune essence' });
        globalThis.__ok = fn({ op: 'select', token: begin.token, id: 9999, slot: 9 });
    }
}
"#;
    let iso = spawn(src);
    let side = [row("Rune essence", 1436, 25, 3322, 1, false)];
    let mut snap = base();
    snap.trade_offer_open = true;
    snap.trade_partner = Some("bob");
    snap.trade_side = &side;
    post(&iso, &snap);
    tick(&iso, 1);
    let probe = iso.probe("__ok").unwrap();
    assert_eq!(probe.get("kind").and_then(Value::as_str), Some("done"));
    assert_eq!(probe.get("result"), Some(&Value::Bool(false)));
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn offer_n_answers_the_posted_count_dialog_and_settles_on_my_offer() {
    let iso = spawn(OFFER);
    let side = [row("Rune essence", 1436, 25, 3322, 1, false)];
    let mut snap = base();
    snap.trade_offer_open = true;
    snap.trade_partner = Some("bob");
    snap.trade_side = &side;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![inv_button(1436, 1, 3322, 5)],
        "offer n sends Offer X, not a generic if-button"
    );
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);

    snap.tick = 2;
    snap.count_dialog_open = true;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::AnswerCount { value: 10 }]
    );
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);

    let mine = [row("Rune essence", 1436, 10, 3415, 0, false)];
    snap.tick = 3;
    snap.count_dialog_open = false;
    snap.trade_mine = &mine;
    post(&iso, &snap);
    tick(&iso, 3);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(true));
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn offer_n_refuses_an_over_offer_and_does_not_answer_late() {
    let iso = spawn(OFFER);
    let side = [row("Rune essence", 1436, 5, 3322, 1, false)];
    let mut snap = base();
    snap.trade_offer_open = true;
    snap.trade_partner = Some("bob");
    snap.trade_side = &side;
    let _ = iso.probe("globalThis.__n = 10");
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(false));
    assert!(
        iso.drain_interacts().is_empty(),
        "n greater than the selected stack must not send Offer X"
    );
    iso.join();
}

#[test]
fn offer_does_not_answer_count_until_the_dialog_is_posted_open() {
    let iso = spawn(OFFER);
    let side = [row("Rune essence", 1436, 25, 3322, 1, false)];
    let mut snap = base();
    snap.trade_offer_open = true;
    snap.trade_partner = Some("bob");
    snap.trade_side = &side;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts().len(), 1);
    snap.tick = 2;
    post(&iso, &snap);
    tick(&iso, 2);
    assert!(
        iso.drain_interacts().is_empty(),
        "a closed count dialog must not be answered"
    );
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);
    iso.join();
}

#[test]
fn stale_partner_or_closed_screen_cancels_an_outstanding_offer() {
    let iso = spawn(OFFER);
    let side = [row("Rune essence", 1436, 25, 3322, 1, false)];
    let mut snap = base();
    snap.trade_offer_open = true;
    snap.trade_partner = Some("bob");
    snap.trade_side = &side;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts().len(), 1);

    snap.tick = 2;
    snap.trade_partner = Some("eve");
    snap.count_dialog_open = true;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(false));
    assert!(
        iso.drain_interacts().is_empty(),
        "a changed partner must not be answered"
    );
    iso.join();
}

#[test]
fn remove_all_progresses_one_inv_button_at_a_time_until_empty() {
    let iso = spawn(REMOVE);
    let mine = [
        row("Lobster", 379, 1, 3415, 0, false),
        row("Coins", 995, 100, 3415, 1, false),
    ];
    let mut snap = base();
    snap.trade_offer_open = true;
    snap.trade_partner = Some("bob");
    snap.trade_mine = &mine;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![inv_button(379, 0, 3415, 4)],
        "removeAll takes the first mine row with Remove All, no name required"
    );
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);

    let rest = [row("Coins", 995, 100, 3415, 1, false)];
    snap.tick = 2;
    snap.trade_mine = &rest;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.drain_interacts(), vec![inv_button(995, 1, 3415, 4)]);

    snap.tick = 3;
    snap.trade_mine = &[];
    post(&iso, &snap);
    tick(&iso, 3);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(true));
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn accept_uses_the_posted_offer_or_confirm_button() {
    let iso = spawn(ACCEPT);
    let mut snap = base();
    snap.trade_offer_open = true;
    snap.trade_accept_id = 3420;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::IfButton { component_id: 3420 }]
    );
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(true));
    iso.join();

    let iso = spawn(ACCEPT);
    let mut confirm = base();
    confirm.trade_confirm_open = true;
    confirm.trade_accept_id = 3546;
    post(&iso, &confirm);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::IfButton { component_id: 3546 }]
    );
    iso.join();
}

#[test]
fn accept_without_a_screen_or_button_is_false() {
    let iso = spawn(ACCEPT);
    post(&iso, &base());
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(false));
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn decline_presses_the_posted_button_then_falls_back_to_close_modal() {
    let iso = spawn(DECLINE);
    let mut snap = base();
    snap.trade_offer_open = true;
    snap.trade_decline_id = 3422;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::IfButton { component_id: 3422 }]
    );
    assert_eq!(iso.probe("__ok").unwrap(), "ran");

    snap.tick = 2;
    snap.trade_offer_open = false;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.probe("__ok").unwrap(), "done");
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn pause_hold_and_session_reset_abort_a_parked_offer() {
    let iso = spawn(OFFER);
    let side = [row("Rune essence", 1436, 25, 3322, 1, false)];
    let mine = [row("Rune essence", 1436, 10, 3415, 0, false)];
    let mut snap = base();
    snap.trade_offer_open = true;
    snap.trade_partner = Some("bob");
    snap.trade_side = &side;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts().len(), 1);

    iso.pause();
    snap.tick = 2;
    snap.count_dialog_open = true;
    snap.trade_mine = &mine;
    post(&iso, &snap);
    tick(&iso, 2);
    tick(&iso, 3);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Null,
        "Pause must not complete a parked offer"
    );
    iso.resume();

    snap.hold = true;
    snap.tick = 4;
    post(&iso, &snap);
    tick(&iso, 4);
    tick(&iso, 5);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Null,
        "Guardian hold must freeze the parked offer"
    );

    iso.reset_session_work();
    snap.hold = false;
    snap.tick = 6;
    post(&iso, &snap);
    tick(&iso, 6);
    tick(&iso, 7);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(false));
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}
