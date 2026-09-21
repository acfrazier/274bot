//! `Equipment.items()` must project posted row `id` (RangingGuild bronzeWorn).

use script::isolate_fb::{ItemRowInput, ReachViewInput, SnapshotInput};
use script::{LoadIsolate, LoadShape};

const HARNESS: &str = r#"
import { Equipment } from '../../api/equipment/Equipment.js';
import { Inventory } from '../../api/inventory/Inventory.js';

const BRONZE_ARROW = 882;

function rangingBronzeWorn() {
    const worn = Equipment.items();
    return worn
        .filter((i) => i.id === BRONZE_ARROW)
        .reduce((n, i) => n + i.count, 0);
}

function rangingBronzeHeld() {
    return Inventory.countById(BRONZE_ARROW);
}

export default class T extends LoopingBot {
    loop() {
        const worn = Equipment.items();
        globalThis.__probe = {
            bronzeWorn: rangingBronzeWorn(),
            bronzeHeld: rangingBronzeHeld(),
            containsBow: Equipment.contains('Magic shortbow'),
            containsBronze: Equipment.contains('Bronze arrow'),
            names: worn.map((i) => i.name),
            counts: worn.map((i) => i.count),
        };
    }
}
"#;

fn item(name: &'static str, id: i32, count: i32) -> ItemRowInput<'static> {
    ItemRowInput {
        name: Some(name),
        count,
        id,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: -1,
        slot: -1,
    }
}

fn snapshot<'a>(
    inv: &'a [ItemRowInput<'a>],
    equipment: &'a [ItemRowInput<'a>],
) -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: None,
        ingame: true,
        inv,
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
        equipment,
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
        self_target_kind: 0,
        self_target_index: -1,
        widgets: &[],
    }
}

fn post_snapshot(iso: &LoadIsolate, input: &SnapshotInput<'_>) {
    iso.post_snapshot(script::isolate_fb::encode_snapshot(input));
}

#[test]
fn equipment_items_id_yields_ranging_bronze_worn_from_posted_rows() {
    let iso = LoadIsolate::spawn(HARNESS.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let equipment = [
        item("Magic shortbow", 861, 1),
        item("Bronze arrow", 882, 10),
    ];
    post_snapshot(&iso, &snapshot(&[], &equipment));
    iso.on_game_tick(1);
    let probe = iso.probe("__probe").unwrap();
    assert_eq!(
        probe["bronzeWorn"], 10,
        "RangingGuild view filters worn rows by id 882"
    );
    assert_eq!(probe["bronzeHeld"], 0);
    assert_eq!(probe["containsBow"], true);
    assert_eq!(probe["containsBronze"], true);
    assert_eq!(
        probe["names"],
        serde_json::json!(["Magic shortbow", "Bronze arrow"])
    );
    assert_eq!(probe["counts"], serde_json::json!([1, 10]));
    iso.join();
}

#[test]
fn equipment_items_id_inventory_only_does_not_count_as_worn() {
    let iso = LoadIsolate::spawn(HARNESS.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let inv = [item("Bronze arrow", 882, 10)];
    post_snapshot(&iso, &snapshot(&inv, &[]));
    iso.on_game_tick(1);
    let probe = iso.probe("__probe").unwrap();
    assert_eq!(probe["bronzeWorn"], 0);
    assert_eq!(probe["bronzeHeld"], 10);
    assert_eq!(probe["containsBronze"], false);
    iso.join();
}
