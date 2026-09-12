// Task 39 — `ChatDialog.makeX` and `makeFromPanelMax` composed end to end:
// the shim marshals the call, Rust owns the count-dialog and anvil waits,
// and the host re-resolves the exact posted control. A queued click is not
// a completed make.

use script::isolate_fb::{
    encode_snapshot_with_native, ItemRowInput, MakeButtonInput, MakeProductInput, NativeFactsInput,
    ReachViewInput, SnapshotInput, TileInput,
};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};
use serde_json::Value;

fn spawn(src: &str) -> LoadIsolate {
    LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn panel_row<'a>(
    name: &'a str,
    id: i32,
    slot: i32,
    component_id: i32,
    ops: &'a [String],
) -> ItemRowInput<'a> {
    ItemRowInput {
        name: Some(name),
        count: 1,
        id,
        ops,
        noted: false,
        cert: -1,
        component_id,
        slot,
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

fn post(iso: &LoadIsolate, snap: &SnapshotInput<'_>, main_make: Option<&[ItemRowInput<'_>]>) {
    iso.post_snapshot(encode_snapshot_with_native(
        snap,
        NativeFactsInput {
            main_make,
            ..Default::default()
        },
    ));
}

fn tick(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    let _ = iso.probe("true");
}

const MAKE_X: &str = r#"
import { ChatDialog } from '../../api/ui/dialogue/ChatDialog.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        try {
            globalThis.__ok = await ChatDialog.makeX(globalThis.__match || 'Flax', globalThis.__count ?? 28);
        } catch (e) {
            globalThis.__ok = String(e.message || e);
        }
    }
}
"#;

const PANEL_MAX: &str = r#"
import { ChatDialog } from '../../api/ui/dialogue/ChatDialog.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        try {
            globalThis.__ok = await ChatDialog.makeFromPanelMax(globalThis.__match || 'Dagger');
        } catch (e) {
            globalThis.__ok = String(e.message || e);
        }
    }
}
"#;

const FACTS: &str = r#"
import { ChatDialog } from '../../api/ui/dialogue/ChatDialog.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        try {
            globalThis.__panel = ChatDialog.isMainMakePanel();
            globalThis.__products = ChatDialog.mainMakeProducts();
            globalThis.__ok = true;
        } catch (e) {
            globalThis.__ok = String(e.message || e);
        }
    }
}
"#;

#[test]
fn make_x_clicks_the_posted_button_and_does_not_answer_a_closed_dialog() {
    let iso = spawn(MAKE_X);
    let buttons = [MakeButtonInput {
        qty: -1,
        com_id: 8875,
    }];
    let products = [MakeProductInput {
        object_id: 1777,
        name: "Flax",
        buttons: &buttons,
    }];
    let mut snap = base();
    snap.make_products = &products;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::IfButton { component_id: 8875 }],
        "makeX clicks the posted Make-X component"
    );
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Null,
        "a queued Make-X click is not a completed make"
    );

    snap.tick = 2;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 2);
    assert!(
        iso.drain_interacts().is_empty(),
        "a closed count dialog must not receive Answer-Count"
    );
    iso.join();
}

#[test]
fn make_x_answers_once_the_count_dialog_opens_then_waits_the_menu() {
    let iso = spawn(MAKE_X);
    let buttons = [MakeButtonInput {
        qty: -1,
        com_id: 8875,
    }];
    let products = [MakeProductInput {
        object_id: 1779,
        name: "Flax",
        buttons: &buttons,
    }];
    let mut snap = base();
    snap.make_products = &products;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::IfButton { component_id: 8875 }]
    );

    snap.tick = 2;
    snap.count_dialog_open = true;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 2);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::AnswerCount { value: 28 }],
        "one Answer-Count after the posted count dialog opens"
    );

    snap.tick = 3;
    snap.count_dialog_open = false;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 3);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Null,
        "count close is not make-menu close"
    );

    snap.tick = 4;
    snap.make_products = &[];
    post(&iso, &snap, Some(&[]));
    tick(&iso, 4);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Bool(true),
        "makeX completes when the make menu drops"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn make_x_without_a_make_x_button_is_an_explicit_failure() {
    let iso = spawn(MAKE_X);
    let buttons = [MakeButtonInput {
        qty: 10,
        com_id: 40,
    }];
    let products = [MakeProductInput {
        object_id: 1779,
        name: "Flax",
        buttons: &buttons,
    }];
    let mut snap = base();
    snap.make_products = &products;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    let probe = iso.probe("__ok").unwrap();
    assert!(
        probe.as_str().unwrap_or("").contains("not impl"),
        "missing Make-X control stays explicit, got {probe:?}"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn make_x_absent_product_resolves_false_without_a_packet() {
    let iso = spawn(MAKE_X);
    let buttons = [MakeButtonInput {
        qty: -1,
        com_id: 42,
    }];
    let products = [MakeProductInput {
        object_id: 2,
        name: "Cannonball",
        buttons: &buttons,
    }];
    let mut snap = base();
    snap.make_products = &products;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(false));
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn make_from_panel_max_presses_the_largest_posted_make_n() {
    let iso = spawn(PANEL_MAX);
    let ops = [
        "Examine".to_string(),
        "Make 1".to_string(),
        "Make 5".to_string(),
        "Make 10".to_string(),
    ];
    let rows = [panel_row("Bronze dagger", 1205, 0, 1119, &ops)];
    let mut snap = base();
    snap.main_modal_id = 3000;
    post(&iso, &snap, Some(&rows));
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::MakePanel {
            id: 1205,
            slot: 0,
            component: 1119,
            operation: 4,
        }],
        "Make 10 is iop index 3, 1-based operation 4"
    );

    snap.tick = 2;
    snap.main_modal_id = -1;
    post(&iso, &snap, Some(&rows));
    tick(&iso, 2);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(true));
    iso.join();
}

#[test]
fn make_from_panel_max_without_a_make_op_refuses() {
    let iso = spawn(PANEL_MAX);
    let ops = ["Examine".to_string(), "Drop".to_string()];
    let rows = [panel_row("Bronze dagger", 1205, 0, 1119, &ops)];
    let mut snap = base();
    snap.main_modal_id = 3000;
    post(&iso, &snap, Some(&rows));
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(false));
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn make_from_panel_max_without_a_decoded_panel_fails_closed() {
    let iso = spawn(PANEL_MAX);
    let mut snap = base();
    snap.main_modal_id = 3000;
    post(&iso, &snap, None);
    tick(&iso, 1);
    let probe = iso.probe("__ok").unwrap();
    assert!(
        probe.as_str().unwrap_or("").contains("not impl"),
        "an unpublished anvil panel is an explicit failure, got {probe:?}"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn is_main_make_panel_reads_posted_rows_and_not_chat_make_products() {
    let iso = spawn(FACTS);
    let ops = ["Make 10".to_string()];
    let rows = [panel_row("Bronze dagger", 1205, 0, 1119, &ops)];
    let mut snap = base();
    snap.main_modal_id = 3000;
    post(&iso, &snap, Some(&rows));
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(true));
    assert_eq!(iso.probe("__panel").unwrap(), Value::Bool(true));
    let products = iso.probe("__products").unwrap();
    assert_eq!(products[0], "Bronze dagger");
    iso.join();
}

#[test]
fn unpublished_main_make_facts_throw_not_impl() {
    let iso = spawn(FACTS);
    let snap = base();
    post(&iso, &snap, None);
    tick(&iso, 1);
    let probe = iso.probe("__ok").unwrap();
    assert!(
        probe.as_str().unwrap_or("").contains("not impl"),
        "unpublished panel facts stay explicit, got {probe:?}"
    );
    iso.join();
}

#[test]
fn pause_hold_and_session_reset_abort_a_parked_make_x() {
    let iso = spawn(MAKE_X);
    let buttons = [MakeButtonInput {
        qty: -1,
        com_id: 8875,
    }];
    let products = [MakeProductInput {
        object_id: 1779,
        name: "Flax",
        buttons: &buttons,
    }];
    let mut snap = base();
    snap.make_products = &products;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts().len(), 1);

    iso.pause();
    snap.tick = 2;
    snap.count_dialog_open = true;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 2);
    tick(&iso, 3);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Null,
        "Pause must not answer a parked count dialog"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.resume();

    snap.hold = true;
    snap.tick = 4;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 4);
    tick(&iso, 5);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Null,
        "Guardian hold must freeze the parked count wait"
    );
    assert!(iso.drain_interacts().is_empty());

    iso.reset_session_work();
    snap.hold = false;
    snap.tick = 6;
    post(&iso, &snap, Some(&[]));
    tick(&iso, 6);
    tick(&iso, 7);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Bool(false),
        "a reset session returns false"
    );
    assert!(
        iso.drain_interacts().is_empty(),
        "a reset session must not answer later"
    );
    iso.join();
}

#[test]
fn make_one_and_make_from_panel_stay_not_impl() {
    let src = r#"
import { ChatDialog } from '../../api/ui/dialogue/ChatDialog.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__one = null;
        globalThis.__panel = null;
        try { globalThis.__one = await ChatDialog.makeOne('Flax'); }
        catch (e) { globalThis.__one = String(e.message || e); }
        try { globalThis.__panel = await ChatDialog.makeFromPanel('Dagger'); }
        catch (e) { globalThis.__panel = String(e.message || e); }
    }
}
"#;
    let iso = spawn(src);
    post(&iso, &base(), Some(&[]));
    tick(&iso, 1);
    let one = iso.probe("__one").unwrap();
    let panel = iso.probe("__panel").unwrap();
    assert!(
        one.as_str().unwrap_or("").contains("not impl"),
        "makeOne stays not impl, got {one:?}"
    );
    assert!(
        panel.as_str().unwrap_or("").contains("not impl"),
        "makeFromPanel stays not impl, got {panel:?}"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}
