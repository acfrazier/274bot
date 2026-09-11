use script::isolate_fb::{
    decode_interact_batch, encode_interact_batch, ItemRowInput, ReachViewInput, SnapshotInput,
    TileInput,
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
            x: 3269,
            z: 3167,
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
        bank_open: true,
        bank_loaded: true,
        bank_generation: 1,
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
        main_modal_id: 600,
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

const WITHDRAW_OPS: [&str; 5] = [
    "Withdraw 1",
    "Withdraw 5",
    "Withdraw 10",
    "Withdraw All",
    "Withdraw X",
];

fn hide_row<'a>(id: i32, slot: i32, ops: &'a [String]) -> ItemRowInput<'a> {
    ItemRowInput {
        name: Some("Dragonhide"),
        count: 5,
        id,
        ops,
        noted: false,
        cert: -1,
        component_id: 601,
        slot,
    }
}

const SELECT_11: &str = r#"
import { Bank } from '../../api/bank/Bank.js';
import { Input } from '../../input/Input.js';
export default class T extends LoopingBot {
    loop() {
        const item = Bank.items().find((i) => i.id === 11);
        let op = -1;
        if (item) {
            for (let i = 0; i < item.ops.length; i++) {
                const label = item.ops[i];
                if (label && /withdraw[\s-]*x/i.test(String(label))) {
                    op = i + 1;
                    break;
                }
            }
        }
        globalThis.__probe = item && op !== -1
            ? Input.invButton(item.id, item.slot, item.comId, op)
            : false;
    }
}
"#;

fn queue_selected(src: &str, first_then_second: bool) -> Vec<InteractReq> {
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let ops: Vec<String> = WITHDRAW_OPS.iter().map(|s| (*s).to_string()).collect();
    let a = hide_row(10, 0, &ops);
    let b = hide_row(11, 1, &ops);
    let bank = if first_then_second { [a, b] } else { [b, a] };
    let mut snap = base_snapshot();
    snap.bank = &bank;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let probe = iso.probe("__probe").unwrap();
    assert_eq!(probe, true, "selected id 11 Withdraw-X must queue");
    let reqs = iso.drain_interacts();
    iso.join();
    reqs
}

#[test]
fn inv_button_queues_selected_id_with_first_row_other_name_twin() {
    assert_eq!(
        queue_selected(SELECT_11, true),
        vec![InteractReq::InvButton {
            id: 11,
            slot: 1,
            component: 601,
            operation: 5,
            bank_generation: 1,
        }],
        "id 10 first still queues selected 11"
    );
}

#[test]
fn inv_button_queues_selected_id_with_selected_row_first() {
    assert_eq!(
        queue_selected(SELECT_11, false),
        vec![InteractReq::InvButton {
            id: 11,
            slot: 1,
            component: 601,
            operation: 5,
            bank_generation: 1,
        }],
        "id 11 first still queues selected 11"
    );
}

#[test]
fn inv_button_invalid_op_closed_missing_and_forged_send_nothing() {
    let iso = LoadIsolate::spawn(
        r#"
import { Bank } from '../../api/bank/Bank.js';
import { Input } from '../../input/Input.js';
export default class T extends LoopingBot {
    loop() {
        const item = Bank.items().find((i) => i.id === 11);
        globalThis.__invalid = item
            ? Input.invButton(item.id, item.slot, item.comId, 9)
            : false;
        globalThis.__forged = Input.invButton(9999, 1, 601, 5);
        globalThis.__wrongSlot = Input.invButton(11, 0, 601, 5);
    }
}
"#
        .to_string(),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    let ops: Vec<String> = WITHDRAW_OPS.iter().map(|s| (*s).to_string()).collect();
    let bank = [hide_row(10, 0, &ops), hide_row(11, 1, &ops)];
    let mut snap = base_snapshot();
    snap.bank = &bank;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__invalid").unwrap(), false);
    assert_eq!(iso.probe("__forged").unwrap(), false);
    assert_eq!(iso.probe("__wrongSlot").unwrap(), false);
    assert!(
        iso.drain_interacts().is_empty(),
        "invalid/forged/wrong-slot must not queue"
    );
    iso.join();

    let closed = LoadIsolate::spawn(SELECT_11.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let mut shut = base_snapshot();
    shut.bank_open = false;
    shut.bank = &bank;
    post_snapshot_input(&closed, &shut);
    closed.on_game_tick(1);
    assert_eq!(closed.probe("__probe").unwrap(), false);
    assert!(
        closed.drain_interacts().is_empty(),
        "closed bank must not queue"
    );
    closed.join();

    let missing =
        LoadIsolate::spawn(SELECT_11.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let only_first = [hide_row(10, 0, &ops)];
    let mut gone = base_snapshot();
    gone.bank = &only_first;
    post_snapshot_input(&missing, &gone);
    missing.on_game_tick(1);
    assert_eq!(missing.probe("__probe").unwrap(), false);
    assert!(
        missing.drain_interacts().is_empty(),
        "missing selected id must not queue the same-name twin"
    );
    missing.join();
}

#[test]
fn inv_button_does_not_queue_count_answer() {
    let reqs = queue_selected(SELECT_11, true);
    assert!(
        !reqs
            .iter()
            .any(|req| matches!(req, InteractReq::AnswerCount { .. })),
        "catalog owns the later count dialog"
    );
}

#[test]
fn inv_button_round_trips_through_flatbuffer() {
    let reqs = queue_selected(SELECT_11, true);
    let bytes = encode_interact_batch(&reqs);
    let got = decode_interact_batch(&bytes).expect("inv-button batch decodes");
    assert_eq!(got, reqs);
}
