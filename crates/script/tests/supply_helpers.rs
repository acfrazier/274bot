use client::io::ClientRevision;
use script::{LoadIsolate, LoadShape};

fn post_base(iso: &LoadIsolate, tick: u64) {
    let mut input = script::isolate_fb::SnapshotInput {
        tick,
        here: None,
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
        side_tab: -1,
        varps: &[],
        combat_styles: &[],
        run_energy: 0,
        run_enabled: false,
        retaliate_enabled: false,
        my_name: None,
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
        scene_state: 0,
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
        reach: script::isolate_fb::ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
        widgets: &[],
    };
    input.tick = tick;
    iso.post_snapshot(script::isolate_fb::encode_snapshot(&input));
}

fn probe_v2(src: &str, revision: ClientRevision) -> serde_json::Value {
    let data = api::game_data::for_revision(revision).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data)
        .unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let value = iso.probe("globalThis.__probe").unwrap();
    iso.join();
    serde_json::from_str(value.as_str().unwrap()).unwrap()
}

fn probe_compat(src: &str, revision: ClientRevision) -> serde_json::Value {
    let data = api::game_data::for_revision(revision).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(src.into(), LoadShape::CompatClass, vec![], data)
        .unwrap();
    iso.on_game_tick(1);
    let value = iso.probe("__probe").unwrap();
    iso.join();
    serde_json::from_str(value.as_str().unwrap()).unwrap()
}

#[test]
fn native_food_count_hook_is_callable() {
    let iso = LoadIsolate::spawn(
        r#"
export default class T extends LoopingBot {
    loop() {
        const fn = globalThis.__rs2b0t_food_count;
        let n = null;
        let err = '';
        try {
            n = fn([{ name: 'Trout' }], 'Trout');
        } catch (e) {
            err = String(e && (e.message || e));
        }
        globalThis.__probe = JSON.stringify({ typeofFn: typeof fn, n, err });
    }
}
"#
        .into(),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    iso.on_game_tick(1);
    let value: serde_json::Value =
        serde_json::from_str(iso.probe("__probe").unwrap().as_str().unwrap()).unwrap();
    iso.join();
    assert_eq!(value["typeofFn"], "function", "{value:?}");
    assert_eq!(value["err"], "", "{value:?}");
    assert_eq!(value["n"], 1, "{value:?}");
}

#[test]
fn v1_food_count_coercions_and_cake_forms() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let value = probe_compat(
            r#"
import { foodCount } from '../../api/combat/food.js';
export default class T extends LoopingBot {
    loop() {
        const trout = Array.from({ length: 8 }, () => ({ name: 'Trout', count: 99 }));
        globalThis.__probe = JSON.stringify({
            nonArray: foodCount(null, 'Trout'),
            stackIgnored: foodCount([{ name: 'Lobster', count: 10 }], 'Lobster'),
            trout: foodCount(trout, 'Trout'),
            cake: foodCount([
                { name: 'Cake' },
                { name: '2/3 cake' },
                { name: 'Slice of cake' },
                { name: 'Coins' },
            ], 'Cake'),
            oddName: foodCount([{ name: null }], ''),
        });
    }
}
"#,
            revision,
        );
        assert_eq!(value["nonArray"], 0);
        assert_eq!(value["stackIgnored"], 1);
        assert_eq!(value["trout"], 8);
        assert_eq!(value["cake"], 3);
        assert_eq!(value["oddName"], 1);
    }
}

#[test]
fn v1_food_heal_known_and_not_impl() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let value = probe_compat(
            r#"
import { foodHealAmount } from '../../api/combat/food.js';
export default class T extends LoopingBot {
    loop() {
        let unknown = '';
        let ambiguous = '';
        try { foodHealAmount('Not a food'); } catch (e) { unknown = String(e.message || e); }
        try { foodHealAmount('Cabbage'); } catch (e) { ambiguous = String(e.message || e); }
        globalThis.__probe = JSON.stringify({
            bread: foodHealAmount('Bread'),
            anchovies: foodHealAmount('Anchovies'),
            unknown,
            ambiguous,
        });
    }
}
"#,
            revision,
        );
        assert_eq!(value["bread"], 4);
        assert_eq!(value["anchovies"], 3);
        assert!(
            value["unknown"]
                .as_str()
                .unwrap_or("")
                .contains("not impl"),
            "{value:?}"
        );
        assert!(
            value["ambiguous"]
                .as_str()
                .unwrap_or("")
                .contains("not impl"),
            "{value:?}"
        );
    }
}

#[test]
fn v2_food_and_runes_contracts() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let value = probe_v2(
            r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__probe = JSON.stringify({
    foodCount: api.foodCount({ items: [], foodName: 'Shark' }),
    badFood: api.foodCount({ items: 'nope', foodName: 'Shark' }),
    heal: api.foodHealAmount({ foodName: 'Bread' }),
    unknownHeal: api.foodHealAmount({ foodName: 'Cabbage' }),
    windExact: api.combatKeepNames({ food: 'Lobster', style: 'mage', spell: 'Wind Strike' }),
    windLower: api.combatKeepNames({ food: 'Lobster', style: 'mage', spell: 'wind strike' }),
    staff: api.runesPerCast({ spellName: 'Wind Strike', wielded: ['Staff of air'] }),
    unknownSpell: api.runesPerCast({ spellName: 'Not a spell', wielded: [] }),
    invRuneNamesIgnored: api.runesPerCast({
      spellName: 'Wind Strike',
      wielded: ['Staff of air', 'Mind rune', 'Air rune'],
    }),
  });
}
"#,
            revision,
        );
        assert_eq!(value["foodCount"]["ok"], true);
        assert_eq!(value["badFood"]["error"], "invalid-args");
        assert_eq!(value["heal"]["ok"], true);
        assert_eq!(value["heal"]["value"], 4);
        assert_eq!(value["unknownHeal"]["error"], "unknown-food");
        assert_eq!(
            value["windExact"]["value"],
            serde_json::json!(["lobster", "Mind rune", "Air rune"])
        );
        assert_eq!(value["windLower"]["value"], serde_json::json!(["lobster"]));
        assert_eq!(
            value["staff"]["value"],
            serde_json::json!([{ "rune": "Mind rune", "count": 1 }])
        );
        assert!(value["unknownSpell"]["value"].is_null());
        assert_eq!(value["invRuneNamesIgnored"]["value"], value["staff"]["value"]);
    }
}

#[test]
fn v2_escape_runes_seven_ids_and_errors() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let value = probe_v2(
            r#"
export const apiVersion = 2;
export function tick(api) {
  const ids = ['varrock','lumbridge','falador','camelot','ardougne','watchtower','trollheim'];
  const rows = Object.fromEntries(ids.map((id) => [id, api.escapeRunesFor({ id })]));
  globalThis.__probe = JSON.stringify({
    ...rows,
    case: api.escapeRunesFor({ id: 'Varrock' }),
    space: api.escapeRunesFor({ id: ' varrock ' }),
    unknown: api.escapeRunesFor({ id: 'narnia' }),
    badType: api.escapeRunesFor({ id: 1 }),
  });
}
"#,
            revision,
        );
        let varrock = &value["varrock"];
        assert_eq!(varrock["ok"], true);
        assert_eq!(varrock["value"]["level"], 25);
        assert_eq!(varrock["value"]["label"], "Varrock teleport");
        assert_eq!(
            varrock["value"]["runes"],
            serde_json::json!([
                { "rune": "Fire rune", "count": 1 },
                { "rune": "Air rune", "count": 3 },
                { "rune": "Law rune", "count": 1 }
            ])
        );
        assert_eq!(value["case"]["error"], "unknown-id");
        assert_eq!(value["space"]["error"], "unknown-id");
        assert_eq!(value["unknown"]["error"], "unknown-id");
        assert_eq!(value["badType"]["error"], "invalid-args");
        for id in [
            "lumbridge", "falador", "camelot", "ardougne", "watchtower", "trollheim",
        ] {
            assert_eq!(value[id]["ok"], true, "{id}");
        }
    }
}

#[test]
fn v2_missing_selected_data_without_cache() {
    let iso = LoadIsolate::spawn(
        r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__probe = JSON.stringify({
    food: api.foodCount({ items: [], foodName: 'Shark' }),
    escape: api.escapeRunesFor({ id: 'varrock' }),
  });
}
"#
        .into(),
        LoadShape::NativeTick,
        vec![],
    )
    .unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let value: serde_json::Value =
        serde_json::from_str(iso.probe("globalThis.__probe").unwrap().as_str().unwrap()).unwrap();
    iso.join();
    assert_eq!(value["food"]["error"], "missing-selected-data");
    assert_eq!(value["escape"]["error"], "missing-selected-data");
}
