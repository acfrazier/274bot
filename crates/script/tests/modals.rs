//! Modals.close / closeIfOpen: shim marshals into the Rust wait; queued
//! close-modal is not a settled close. Identity change settles; a newer
//! root is never closed.

use script::isolate_fb::{encode_snapshot, ReachViewInput, SnapshotInput};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};
use serde_json::Value;

fn spawn(src: &str) -> LoadIsolate {
    LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn base<'a>() -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: None,
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
        widgets: &[],
    }
}

fn post(iso: &LoadIsolate, snap: &SnapshotInput<'_>) {
    iso.post_snapshot(encode_snapshot(snap));
}

fn tick(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    let _ = iso.probe("true");
}

const CLOSE: &str = r#"
import { Modals } from '../../api/ui/widgets/Modals.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        globalThis.__type = null;
        try {
            const v = await Modals.close();
            globalThis.__ok = v;
            globalThis.__type = typeof v;
        } catch (e) {
            globalThis.__ok = String(e.message || e);
            globalThis.__type = 'threw';
        }
    }
}
"#;

const CLOSE_IF_OPEN: &str = r#"
import { Modals } from '../../api/ui/widgets/Modals.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = 'sentinel';
        globalThis.__type = null;
        try {
            const v = await Modals.closeIfOpen();
            globalThis.__ok = v;
            globalThis.__type = typeof v;
        } catch (e) {
            globalThis.__ok = String(e.message || e);
            globalThis.__type = 'threw';
        }
    }
}
"#;

#[test]
fn close_absent_returns_true_and_queues_nothing() {
    let iso = spawn(CLOSE);
    let snap = base();
    post(&iso, &snap);
    tick(&iso, 1);
    assert!(
        iso.drain_interacts().is_empty(),
        "no modal must not emit close-modal"
    );
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(true));
    assert_eq!(iso.probe("__type").unwrap(), "boolean");
    iso.join();
}

#[test]
fn close_root_a_to_closed_settles_true_with_one_action() {
    let iso = spawn(CLOSE);
    let mut snap = base();
    snap.main_modal_id = 6675;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![InteractReq::CloseModal]);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Null,
        "queued close-modal is not a settled close"
    );

    snap.tick = 2;
    snap.main_modal_id = -1;
    post(&iso, &snap);
    tick(&iso, 2);
    assert!(iso.drain_interacts().is_empty(), "one action only");
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(true));
    iso.join();
}

#[test]
fn close_root_a_to_root_b_settles_without_closing_the_newer_root() {
    let iso = spawn(CLOSE);
    let mut snap = base();
    snap.main_modal_id = 6675;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![InteractReq::CloseModal]);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);

    snap.tick = 2;
    snap.main_modal_id = 3824;
    post(&iso, &snap);
    tick(&iso, 2);
    assert!(
        iso.drain_interacts().is_empty(),
        "identity change settles; do not close the newer root"
    );
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(true));
    iso.join();
}

#[test]
fn close_unchanged_root_times_out_false() {
    let iso = spawn(CLOSE);
    let mut snap = base();
    snap.main_modal_id = 6675;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![InteractReq::CloseModal]);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);

    snap.tick = 2;
    post(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Null,
        "unchanged root is not success"
    );
    assert!(iso.drain_interacts().is_empty());

    std::thread::sleep(std::time::Duration::from_millis(3_100));
    snap.tick = 3;
    post(&iso, &snap);
    tick(&iso, 3);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Bool(false));
    assert!(
        iso.drain_interacts().is_empty(),
        "timeout must not re-close"
    );
    iso.join();
}

#[test]
fn close_if_open_absent_is_void_and_queues_nothing() {
    let iso = spawn(CLOSE_IF_OPEN);
    let snap = base();
    post(&iso, &snap);
    tick(&iso, 1);
    assert!(iso.drain_interacts().is_empty());
    assert_eq!(iso.probe("__type").unwrap(), "undefined");
    iso.join();
}

#[test]
fn close_if_open_present_waits_then_returns_void() {
    let iso = spawn(CLOSE_IF_OPEN);
    let mut snap = base();
    snap.main_modal_id = 6675;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![InteractReq::CloseModal]);
    assert_eq!(
        iso.probe("__type").unwrap(),
        Value::Null,
        "still parked; void is not reported until settle"
    );

    snap.tick = 2;
    snap.main_modal_id = -1;
    post(&iso, &snap);
    tick(&iso, 2);
    assert!(iso.drain_interacts().is_empty());
    assert_eq!(iso.probe("__type").unwrap(), "undefined");
    iso.join();
}

#[test]
fn pause_hold_and_reset_do_not_emit_a_future_close() {
    let iso = spawn(CLOSE);
    let mut snap = base();
    snap.main_modal_id = 6675;
    post(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![InteractReq::CloseModal]);

    iso.pause();
    snap.tick = 2;
    snap.main_modal_id = -1;
    post(&iso, &snap);
    tick(&iso, 2);
    tick(&iso, 3);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);
    assert!(iso.drain_interacts().is_empty());
    iso.resume();

    snap.main_modal_id = 6675;
    snap.hold = true;
    snap.tick = 4;
    post(&iso, &snap);
    tick(&iso, 4);
    tick(&iso, 5);
    assert_eq!(iso.probe("__ok").unwrap(), Value::Null);
    assert!(
        iso.drain_interacts().is_empty(),
        "hold must not emit another close"
    );

    iso.reset_session_work();
    snap.hold = false;
    snap.main_modal_id = -1;
    snap.tick = 6;
    post(&iso, &snap);
    tick(&iso, 6);
    tick(&iso, 7);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        Value::Bool(false),
        "reset aborts; do not fake a successful close"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}
