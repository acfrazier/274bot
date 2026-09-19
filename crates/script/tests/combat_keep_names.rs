use script::isolate_fb::{ItemRowInput, ReachViewInput, SnapshotInput};
use script::{LoadIsolate, LoadShape};

fn item(name: &'static str, id: i32, slot: i32) -> ItemRowInput<'static> {
    ItemRowInput {
        name: Some(name),
        count: 1,
        id,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: -1,
        slot,
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
        widgets: &[],
    }
}

#[test]
fn snapshot_loot_and_equipment_are_not_implicitly_kept() {
    let src = r#"
import { combatKeepNames } from '../../api/combat/keepList.js';
import { foodForms } from '../../api/combat/food.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = JSON.stringify({
            forms: foodForms('Lobster'),
            names: combatKeepNames({
                food: 'Lobster',
                style: 'melee',
                extra: ['Coins'],
            }),
        });
    }
}
"#;
    let data =
        api::game_data::for_revision(client::io::ClientRevision::R274).expect("274 game data");
    let iso = LoadIsolate::spawn_with_game_data(src.into(), LoadShape::CompatClass, vec![], data)
        .unwrap();
    let inv = [item("Big bones", 532, 0), item("Lobster", 379, 1)];
    let equipment = [item("Rune scimitar", 1333, 3)];
    iso.post_snapshot(script::isolate_fb::encode_snapshot(&snapshot(
        &inv, &equipment,
    )));
    iso.on_game_tick(1);
    let value = iso.probe("__probe").unwrap();
    iso.join();
    let result: serde_json::Value = serde_json::from_str(value.as_str().unwrap()).unwrap();
    assert_eq!(result["forms"], serde_json::json!(["lobster"]));
    assert_eq!(result["names"], serde_json::json!(["lobster", "Coins"]));
    assert!(!result["names"]
        .as_array()
        .unwrap()
        .iter()
        .any(|name| { name == "Big bones" || name == "Rune scimitar" }));
}

#[test]
fn selected_revision_facts_cover_fire_giant_green_dragon_and_cake_shapes() {
    let src = r#"
import { combatKeepNames } from '../../api/combat/keepList.js';
import { foodForms } from '../../api/combat/food.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = JSON.stringify({
            cakeForms: foodForms('Cake'),
            cakeKeep: combatKeepNames({ food: 'Cake', style: 'melee' }),
            fire: combatKeepNames({
                food: 'Lobster',
                style: 'melee',
                extra: ['Amulet of glory', 'Rope'],
            }),
            green: combatKeepNames({
                food: 'Lobster',
                style: 'mage',
                spell: 'Wind Strike',
                extra: [],
            }),
        });
    }
}
"#;
    let inv = [
        item("Big bones", 532, 0),
        item("Chaos rune", 562, 1),
        item("Lobster", 379, 2),
    ];
    let equipment = [item("Rune scimitar", 1333, 3)];
    for revision in [
        client::io::ClientRevision::R274,
        client::io::ClientRevision::R289,
    ] {
        let data = api::game_data::for_revision(revision).expect("selected game data");
        let iso =
            LoadIsolate::spawn_with_game_data(src.into(), LoadShape::CompatClass, vec![], data)
                .unwrap();
        iso.post_snapshot(script::isolate_fb::encode_snapshot(&snapshot(
            &inv, &equipment,
        )));
        iso.on_game_tick(1);
        let value = iso.probe("__probe").unwrap();
        iso.join();
        let result: serde_json::Value = serde_json::from_str(value.as_str().unwrap()).unwrap();
        assert_eq!(result["cakeKeep"], result["cakeForms"]);
        assert_eq!(
            result["fire"],
            serde_json::json!(["lobster", "Amulet of glory", "Rope"])
        );
        assert_eq!(
            result["green"],
            serde_json::json!(["lobster", "Mind rune", "Air rune"])
        );
    }
}

#[test]
fn missing_selected_data_throws_instead_of_returning_an_incomplete_list() {
    let src = r#"
import { combatKeepNames } from '../../api/combat/keepList.js';
export default class T extends LoopingBot {
    loop() {
        try {
            combatKeepNames({ food: 'Lobster', style: 'melee' });
            globalThis.__probe = 'unexpected success';
        } catch (error) {
            globalThis.__probe = String(error && error.message);
        }
    }
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    let error = iso.probe("__probe").unwrap();
    iso.join();
    assert!(
        error
            .as_str()
            .unwrap()
            .contains("combatKeepNames requires selected game data"),
        "unexpected error: {error}"
    );
}
