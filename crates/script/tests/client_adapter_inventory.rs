// ClientAdapter.inventory is a thin mapping onto posted snap().inv
// rows. It does not read bank, equipment or bank-side, invent ops, or
// hardcode a valid inventory component.

use script::isolate_fb::{ItemRowInput, ReachViewInput, SnapshotInput, TileInput};
use script::{LoadIsolate, LoadShape};

fn post_snapshot_input(iso: &LoadIsolate, input: &SnapshotInput<'_>) {
    iso.post_snapshot(script::isolate_fb::encode_snapshot(input));
}

fn base_snapshot<'a>() -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: Some(TileInput {
            x: 3013,
            z: 3355,
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
        side_tab: 3,
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

fn row<'a>(
    name: Option<&'a str>,
    id: i32,
    count: i32,
    slot: i32,
    component_id: i32,
    ops: &'a [String],
    noted: bool,
) -> ItemRowInput<'a> {
    ItemRowInput {
        name,
        count,
        id,
        ops,
        noted,
        cert: -1,
        component_id,
        slot,
    }
}

fn spawn(src: &str) -> LoadIsolate {
    LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn probe_src(body: &str) -> String {
    format!(
        r#"
import {{ reader }} from '../../adapter/ClientAdapter.js';
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
fn sparse_slots_keep_duplicate_names_and_distinct_ids() {
    let iso = spawn(&probe_src(
        r#"
        globalThis.__items = reader.inventory();
        globalThis.__size = reader.inventorySize();
        "#,
    ));
    let wear = ["Wear".to_string()];
    let empty_ops: [String; 0] = [];
    let inv = [
        row(Some("Air talisman"), 1438, 1, 0, 3214, &wear, false),
        row(Some("Air talisman"), 2691, 1, 27, 3214, &empty_ops, false),
        row(Some("Dwarf remains"), 0, 1, 4, 3214, &empty_ops, false),
        row(None, 995, 25, 8, 3214, &empty_ops, false),
        row(Some("Air rune"), 556, 400, 12, 3214, &empty_ops, true),
    ];
    let mut snap = base_snapshot();
    snap.inv = &inv;
    let value = tick_probe(&iso, &snap, 1, "__items");
    assert_eq!(iso.probe("__size").unwrap(), 28);
    assert_eq!(value.as_array().map(Vec::len), Some(5));
    assert_eq!(value[0]["slot"], 0);
    assert_eq!(value[0]["id"], 1438);
    assert_eq!(value[0]["name"], "Air talisman");
    assert_eq!(value[1]["slot"], 27);
    assert_eq!(value[1]["id"], 2691);
    assert_eq!(value[1]["name"], "Air talisman");
    assert_eq!(
        value[2]["id"], 0,
        "native-valid item id 0 must not be treated as empty"
    );
    assert_eq!(value[2]["slot"], 4);
    assert_eq!(value[2]["name"], "Dwarf remains");
    assert_eq!(value[3]["name"], serde_json::Value::Null);
    assert_eq!(value[3]["id"], 995);
    assert_eq!(value[3]["slot"], 8);
    assert_eq!(value[4]["id"], 556);
    assert_eq!(value[4]["count"], 400);
    assert_eq!(value[4]["name"], "Air rune");
    assert!(
        iso.drain_interacts().is_empty(),
        "inventory is a sync read, no queued command"
    );
    iso.join();
}

#[test]
fn counts_ops_and_component_identity_are_posted_not_invented() {
    let iso = spawn(&probe_src(
        r#"
        globalThis.__items = reader.inventory();
        "#,
    ));
    let wear = ["Wear".to_string()];
    let inv = [row(Some("Air talisman"), 1438, 1, 0, 3214, &wear, false)];
    let mut snap = base_snapshot();
    snap.inv = &inv;
    let value = tick_probe(&iso, &snap, 1, "__items");
    assert_eq!(value[0]["count"], 1);
    assert_eq!(value[0]["comId"], 3214);
    assert_eq!(value[0]["ops"][0], "Wear");
    assert_eq!(
        value[0]["ops"].as_array().map(Vec::len),
        Some(1),
        "must not invent Drop or pad ops"
    );

    iso.probe("delete globalThis.__rs2b0t_host.snapshot.inv[0].component_id;true")
        .unwrap();
    iso.on_game_tick(2);
    let _ = iso.probe("true");
    let missing = iso.probe("__items").unwrap();
    assert_eq!(
        missing[0]["comId"], -1,
        "missing component identity is the unavailable sentinel, not a hardcoded valid component"
    );
    assert_eq!(missing[0]["id"], 1438);
    iso.join();
}

#[test]
fn empty_inventory_does_not_read_bank_or_equipment() {
    let iso = spawn(&probe_src(
        r#"
        globalThis.__probe = {
            items: reader.inventory(),
            size: reader.inventorySize(),
        };
        "#,
    ));
    let withdraw = [
        "Withdraw 1".to_string(),
        "Withdraw 5".to_string(),
        "Withdraw 10".to_string(),
        "Withdraw All".to_string(),
        "Withdraw X".to_string(),
    ];
    let wield = ["Wield".to_string()];
    let bank = [row(Some("Cow hide"), 1739, 4000, 0, 5382, &withdraw, false)];
    let equipment = [row(Some("Bronze sword"), 1277, 1, 3, 1688, &wield, false)];
    let mut snap = base_snapshot();
    snap.bank = &bank;
    snap.bank_open = true;
    snap.bank_loaded = true;
    snap.equipment = &equipment;
    let value = tick_probe(&iso, &snap, 1, "__probe");
    assert_eq!(
        value["items"].as_array().map(Vec::len),
        Some(0),
        "ordinary inventory is empty even when bank and equipment have rows"
    );
    assert_eq!(value["size"], 28);
    iso.join();
}

#[test]
fn caller_mutation_does_not_corrupt_posted_snapshot() {
    let iso = spawn(&probe_src(
        r#"
        const first = reader.inventory();
        first[0].id = 1;
        first[0].name = 'forged';
        first[0].count = 99;
        first[0].comId = 3214;
        first[0].ops.push('Drop');
        first.pop();
        globalThis.__after = reader.inventory();
        "#,
    ));
    let wear = ["Wear".to_string()];
    let empty_ops: [String; 0] = [];
    let inv = [
        row(Some("Air talisman"), 1438, 1, 0, 3214, &wear, false),
        row(Some("Air rune"), 556, 25, 3, 3214, &empty_ops, false),
    ];
    let mut snap = base_snapshot();
    snap.inv = &inv;
    let after = tick_probe(&iso, &snap, 1, "__after");
    assert_eq!(after.as_array().map(Vec::len), Some(2));
    assert_eq!(after[0]["id"], 1438);
    assert_eq!(after[0]["name"], "Air talisman");
    assert_eq!(after[0]["count"], 1);
    assert_eq!(after[0]["comId"], 3214);
    assert_eq!(after[0]["ops"].as_array().map(Vec::len), Some(1));
    assert_eq!(after[0]["ops"][0], "Wear");
    assert_eq!(after[1]["id"], 556);
    iso.join();
}

#[test]
fn omitted_delta_keeps_last_inventory_replacement_and_clear_update() {
    let iso = spawn(&probe_src(
        r#"
        globalThis.__items = reader.inventory();
        "#,
    ));
    let wear = ["Wear".to_string()];
    let empty_ops: [String; 0] = [];
    let first_inv = [row(Some("Air talisman"), 1438, 1, 0, 3214, &wear, false)];
    let mut snap = base_snapshot();
    snap.inv = &first_inv;
    let (keyframe, fp) = script::isolate_fb::encode_snapshot_delta(None, &snap, false);
    iso.post_snapshot(keyframe);
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    assert_eq!(iso.probe("__items").unwrap()[0]["id"], 1438);

    snap.tick = 2;
    let (delta, fp2) = script::isolate_fb::encode_snapshot_delta(Some(&fp), &snap, false);
    let omitted = script::isolate_fb::decode_snapshot(&delta).expect("delta");
    assert!(!omitted.has_inv(), "unchanged inv omitted");
    iso.post_snapshot(delta);
    iso.on_game_tick(2);
    let _ = iso.probe("true");
    assert_eq!(
        iso.probe("__items").unwrap()[0]["id"],
        1438,
        "omitted delta keeps last posted inventory"
    );

    snap.tick = 3;
    let replaced = [row(Some("Air rune"), 556, 25, 5, 3214, &empty_ops, false)];
    snap.inv = &replaced;
    let (next, fp3) = script::isolate_fb::encode_snapshot_delta(Some(&fp2), &snap, false);
    iso.post_snapshot(next);
    iso.on_game_tick(3);
    let _ = iso.probe("true");
    let got = iso.probe("__items").unwrap();
    assert_eq!(got.as_array().map(Vec::len), Some(1));
    assert_eq!(got[0]["id"], 556);
    assert_eq!(got[0]["slot"], 5);
    assert_eq!(got[0]["count"], 25);

    snap.tick = 4;
    snap.inv = &[];
    let (cleared, _) = script::isolate_fb::encode_snapshot_delta(Some(&fp3), &snap, false);
    iso.post_snapshot(cleared);
    iso.on_game_tick(4);
    let _ = iso.probe("true");
    assert_eq!(
        iso.probe("__items").unwrap().as_array().map(Vec::len),
        Some(0),
        "explicit empty inventory replaces the previous rows"
    );
    iso.join();
}

#[test]
fn two_isolates_do_not_exchange_inventory_rows() {
    let src = probe_src(
        r#"
        globalThis.__items = reader.inventory();
        "#,
    );
    let a = spawn(&src);
    let b = spawn(&src);
    let wear = ["Wear".to_string()];
    let empty_ops: [String; 0] = [];
    let inv_a = [row(Some("Air talisman"), 1438, 1, 0, 3214, &wear, false)];
    let inv_b = [row(Some("Air rune"), 556, 25, 3, 3214, &empty_ops, false)];
    let mut snap_a = base_snapshot();
    snap_a.inv = &inv_a;
    let mut snap_b = base_snapshot();
    snap_b.inv = &inv_b;
    post_snapshot_input(&a, &snap_a);
    post_snapshot_input(&b, &snap_b);
    a.on_game_tick(1);
    b.on_game_tick(1);
    let pa = a.probe("__items").unwrap();
    let pb = b.probe("__items").unwrap();
    assert_eq!(pa.as_array().map(Vec::len), Some(1));
    assert_eq!(pb.as_array().map(Vec::len), Some(1));
    assert_eq!(pa[0]["id"], 1438);
    assert_eq!(pa[0]["slot"], 0);
    assert_eq!(
        pb[0]["id"], 556,
        "slot B must not inherit slot A's inventory"
    );
    assert_eq!(pb[0]["slot"], 3);
    a.join();
    b.join();
}
