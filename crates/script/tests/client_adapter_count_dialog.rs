// ClientAdapter.countDialogOpen / answerCountDialog are thin mappings
// onto the posted native count_dialog_open fact and the existing
// answer-count command. Queue admission is not a completed withdrawal.

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
        bank_generation: 3,
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

fn hide_row<'a>(ops: &'a [String]) -> ItemRowInput<'a> {
    ItemRowInput {
        name: Some("Cow hide"),
        count: 4000,
        id: 1739,
        ops,
        noted: false,
        cert: -1,
        component_id: 5382,
        slot: 0,
    }
}

fn spawn(src: &str) -> LoadIsolate {
    LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn probe_src(body: &str) -> String {
    format!(
        r#"
import {{ reader, actions }} from '../../adapter/ClientAdapter.js';
export default class T extends LoopingBot {{
    loop() {{
        {body}
    }}
}}
"#
    )
}

fn tick_probe(
    iso: &LoadIsolate,
    snap: &SnapshotInput<'_>,
    tick: u64,
    expr: &str,
) -> serde_json::Value {
    post_snapshot_input(iso, snap);
    iso.on_game_tick(tick);
    let _ = iso.probe("true");
    iso.probe(expr).unwrap()
}

#[test]
fn count_dialog_open_follows_closed_open_closed_posts() {
    let iso = spawn(&probe_src(
        r#"
        globalThis.__open = reader.countDialogOpen();
        "#,
    ));
    let mut snap = base_snapshot();
    assert_eq!(
        tick_probe(&iso, &snap, 1, "__open"),
        false,
        "posted false is closed state, not a missing-op throw"
    );
    assert!(iso.drain_interacts().is_empty());

    snap.tick = 2;
    snap.count_dialog_open = true;
    assert_eq!(tick_probe(&iso, &snap, 2, "__open"), true);

    snap.tick = 3;
    snap.count_dialog_open = false;
    assert_eq!(
        tick_probe(&iso, &snap, 3, "__open"),
        false,
        "a later closed post must not keep the previous open"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn count_dialog_open_rejects_missing_and_malformed_as_closed() {
    let iso = spawn(&probe_src(
        r#"
        globalThis.__open = reader.countDialogOpen();
        "#,
    ));
    let mut snap = base_snapshot();
    snap.count_dialog_open = true;
    assert_eq!(tick_probe(&iso, &snap, 1, "__open"), true);

    iso.probe("globalThis.__rs2b0t_host.snapshot.count_dialog_open=1;true")
        .unwrap();
    iso.on_game_tick(2);
    let _ = iso.probe("true");
    assert_eq!(
        iso.probe("__open").unwrap(),
        false,
        "truthy non-boolean must not report an open dialog"
    );

    iso.probe("globalThis.__rs2b0t_host.snapshot.count_dialog_open='true';true")
        .unwrap();
    iso.on_game_tick(3);
    let _ = iso.probe("true");
    assert_eq!(iso.probe("__open").unwrap(), false);

    iso.probe("delete globalThis.__rs2b0t_host.snapshot.count_dialog_open;true")
        .unwrap();
    iso.on_game_tick(4);
    let _ = iso.probe("true");
    assert_eq!(
        iso.probe("__open").unwrap(),
        false,
        "missing native fact is closed, not a throw"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn answer_count_queues_valid_integer_on_open_dialog() {
    let iso = spawn(&probe_src(
        r#"
        globalThis.__open = reader.countDialogOpen();
        globalThis.__ans = actions.answerCountDialog(4000);
        "#,
    ));
    let mut snap = base_snapshot();
    snap.count_dialog_open = true;
    assert_eq!(tick_probe(&iso, &snap, 1, "__open"), true);
    assert_eq!(iso.probe("__ans").unwrap(), true);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::AnswerCount { value: 4000 }],
        "valid count queues the existing answer-count command"
    );
    iso.join();
}

#[test]
fn answer_count_zero_is_a_supported_integer() {
    let iso = spawn(&probe_src(
        r#"
        globalThis.__ans = actions.answerCountDialog(0);
        "#,
    ));
    let mut snap = base_snapshot();
    snap.count_dialog_open = true;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    assert_eq!(iso.probe("__ans").unwrap(), true);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::AnswerCount { value: 0 }]
    );
    iso.join();
}

#[test]
fn answer_count_invalid_values_do_not_dispatch() {
    let iso = spawn(&probe_src(
        r#"
        globalThis.__probe = {
            neg: actions.answerCountDialog(-1),
            float: actions.answerCountDialog(1.5),
            nan: actions.answerCountDialog(Number.NaN),
            inf: actions.answerCountDialog(Number.POSITIVE_INFINITY),
            str: actions.answerCountDialog('4000'),
            missing: actions.answerCountDialog(),
            big: actions.answerCountDialog(2147483648),
        };
        "#,
    ));
    let mut snap = base_snapshot();
    snap.count_dialog_open = true;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    let value = iso.probe("__probe").unwrap();
    assert_eq!(value["neg"], false);
    assert_eq!(value["float"], false);
    assert_eq!(value["nan"], false);
    assert_eq!(value["inf"], false);
    assert_eq!(value["str"], false);
    assert_eq!(value["missing"], false);
    assert_eq!(value["big"], false);
    assert!(
        iso.drain_interacts().is_empty(),
        "invalid values must not queue answer-count"
    );
    iso.join();
}

#[test]
fn answer_count_does_not_queue_when_dialog_is_closed() {
    let iso = spawn(&probe_src(
        r#"
        globalThis.__open = reader.countDialogOpen();
        globalThis.__ans = actions.answerCountDialog(4000);
        "#,
    ));
    let snap = base_snapshot();
    assert_eq!(tick_probe(&iso, &snap, 1, "__open"), false);
    assert_eq!(iso.probe("__ans").unwrap(), false);
    assert!(
        iso.drain_interacts().is_empty(),
        "no native dialog must not answer"
    );
    iso.join();
}

#[test]
fn tanner_inv_button_then_posted_dialog_then_count_answer() {
    let iso = spawn(
        r#"
import { Bank } from '../../api/bank/Bank.js';
import { Input } from '../../input/Input.js';
import { reader, actions } from '../../adapter/ClientAdapter.js';
export default class T extends LoopingBot {
    loop() {
        if (!globalThis.__phase) {
            const item = Bank.items().find((i) => i.id === 1739);
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
            globalThis.__btn = item && op !== -1
                ? Input.invButton(item.id, item.slot, item.comId, op)
                : false;
            globalThis.__open1 = reader.countDialogOpen();
            globalThis.__ans1 = actions.answerCountDialog(4000);
            globalThis.__phase = 1;
            return;
        }
        globalThis.__open2 = reader.countDialogOpen();
        globalThis.__ans2 = actions.answerCountDialog(4000);
    }
}
"#,
    );
    let ops: Vec<String> = WITHDRAW_OPS.iter().map(|s| (*s).to_string()).collect();
    let bank = [hide_row(&ops)];
    let mut snap = base_snapshot();
    snap.bank = &bank;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    assert_eq!(iso.probe("__btn").unwrap(), true);
    assert_eq!(iso.probe("__open1").unwrap(), false);
    assert_eq!(iso.probe("__ans1").unwrap(), false);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::InvButton {
            id: 1739,
            slot: 0,
            component: 5382,
            operation: 5,
            bank_generation: 3,
        }],
        "selected hide1739 slot0 component5382 operation5; no count answer yet"
    );

    snap.tick = 2;
    snap.count_dialog_open = true;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(2);
    let _ = iso.probe("true");
    assert_eq!(iso.probe("__open2").unwrap(), true);
    assert_eq!(iso.probe("__ans2").unwrap(), true);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::AnswerCount { value: 4000 }],
        "posted dialog then answerCountDialog queues existing answer-count"
    );
    iso.join();
}

#[test]
fn answer_count_round_trips_through_flatbuffer() {
    let iso = spawn(&probe_src(
        r#"
        globalThis.__ans = actions.answerCountDialog(4000);
        "#,
    ));
    let mut snap = base_snapshot();
    snap.count_dialog_open = true;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    let reqs = iso.drain_interacts();
    assert_eq!(reqs, vec![InteractReq::AnswerCount { value: 4000 }]);
    let bytes = encode_interact_batch(&reqs);
    let got = decode_interact_batch(&bytes).expect("answer-count batch decodes");
    assert_eq!(got, reqs);
    iso.join();
}
