use client::io::ClientRevision;
use script::isolate_fb::ItemRowInput;
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

fn post_base(iso: &LoadIsolate, tick: u64) {
    post_inv(iso, tick, &[]);
}

fn post_inv(iso: &LoadIsolate, tick: u64, inv: &[ItemRowInput<'_>]) {
    let mut input = script::isolate_fb::SnapshotInput {
        tick,
        here: None,
        ingame: true,
        inv,
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
        self_target_kind: 0,
        self_target_index: -1,
        widgets: &[],
    };
    input.tick = tick;
    iso.post_snapshot(script::isolate_fb::encode_snapshot(&input));
}

fn probe_v2(src: &str, revision: ClientRevision) -> serde_json::Value {
    let data = api::game_data::for_revision(revision).unwrap();
    let iso =
        LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data).unwrap();
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
fn v1_food_heal_missing_data_explains_content_verification() {
    let iso = LoadIsolate::spawn(
        r#"
import { foodHealAmount } from '../../api/combat/food.js';
export default class T extends LoopingBot {
    loop() {
        let missing = '';
        try { foodHealAmount('Bread'); } catch (e) { missing = String(e.message || e); }
        globalThis.__probe = JSON.stringify({ missing });
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
    assert_eq!(
        value["missing"],
        "game data unavailable: this server's content isn't verified (see profile/engine settings)"
    );
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

/// Frozen `foodHealAmount` (`api/combat/food.ts:86-106`) always answers: an
/// empty, unknown or ambiguous name is `DEFAULT_FOOD_HEAL` (8), a partial
/// name takes the first food it matches, and `shouldEatFood` eats smart on
/// that answer instead of throwing mid-fight.
#[test]
fn v1_food_heal_resolves_every_name_like_frozen() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let value = probe_compat(
            r#"
import { foodHealAmount, shouldEatFood } from '../../api/combat/food.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = JSON.stringify({
            bread: foodHealAmount('Bread'),
            anchovies: foodHealAmount(' ANCHOVIES '),
            empty: foodHealAmount(''),
            unknown: foodHealAmount('Not a food'),
            ambiguous: foodHealAmount('Cabbage'),
            partial: foodHealAmount('Swordf'),
            eatUnknown: shouldEatFood('Not a food', { hp: 10, maxHp: 20, foodCount: 1 }),
            holdUnknown: shouldEatFood('Not a food', { hp: 13, maxHp: 20, foodCount: 1 }),
        });
    }
}
"#,
            revision,
        );
        assert_eq!(value["bread"], 4, "{revision:?}");
        assert_eq!(value["anchovies"], 3, "{revision:?}");
        assert_eq!(value["empty"], 8, "{revision:?}");
        assert_eq!(value["unknown"], 8, "{revision:?}");
        assert_eq!(value["ambiguous"], 8, "{revision:?}");
        assert_eq!(value["partial"], 14, "{revision:?}");
        assert_eq!(value["eatUnknown"], true, "{revision:?}");
        assert_eq!(value["holdUnknown"], false, "{revision:?}");
    }
}

/// The frozen table's form spelling `1/2 pineapple pizza` (the cache spells
/// it `1/2pineapple pizza`) still heals as the pizza (`food.ts:53-54`).
#[test]
fn v1_food_heal_partial_form_takes_the_parent_heal() {
    let value = probe_compat(
        r#"
import { foodHealAmount } from '../../api/combat/food.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = JSON.stringify({
            half: foodHealAmount('1/2 pineapple pizza'),
            whole: foodHealAmount('Pineapple pizza'),
        });
    }
}
"#,
        ClientRevision::R289,
    );
    assert_eq!(value["half"], value["whole"]);
    assert_eq!(value["whole"], 10);
}

/// Frozen `shouldEatToUseFood` (`food.ts:124-143`): a heal of zero never
/// eats above the floor (`:138-141`); the floor and a fitting heal still do.
#[test]
fn v1_should_eat_to_use_food_holds_a_zero_heal() {
    let value = probe_compat(
        r#"
import { shouldEatToUseFood } from '../../api/combat/food.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = JSON.stringify({
            zeroHeal: shouldEatToUseFood({ hp: 10, maxHp: 20, heal: 0, foodCount: 3 }),
            negativeHeal: shouldEatToUseFood({ hp: 10, maxHp: 20, heal: -4, foodCount: 3 }),
            zeroHealAtFloor: shouldEatToUseFood({ hp: 5, maxHp: 20, heal: 0, foodCount: 3 }),
            fits: shouldEatToUseFood({ hp: 10, maxHp: 20, heal: 10, foodCount: 3 }),
            overheals: shouldEatToUseFood({ hp: 11, maxHp: 20, heal: 10, foodCount: 3 }),
            customFloor: shouldEatToUseFood({ hp: 11, maxHp: 20, heal: 10, foodCount: 3, minHp: 11 }),
            noFood: shouldEatToUseFood({ hp: 1, maxHp: 20, heal: 10, foodCount: 0 }),
        });
    }
}
"#,
        ClientRevision::R289,
    );
    assert_eq!(value["zeroHeal"], false);
    assert_eq!(value["negativeHeal"], false);
    assert_eq!(value["zeroHealAtFloor"], true);
    assert_eq!(value["fits"], true);
    assert_eq!(value["overheals"], false);
    assert_eq!(value["customFloor"], true);
    assert_eq!(value["noFood"], false);
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
        assert_eq!(
            value["invRuneNamesIgnored"]["value"],
            value["staff"]["value"]
        );
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
            "lumbridge",
            "falador",
            "camelot",
            "ardougne",
            "watchtower",
            "trollheim",
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
    assert_eq!(
        value["food"]["error"],
        "game data unavailable: this server's content isn't verified (see profile/engine settings)"
    );
    assert_eq!(
        value["escape"]["error"],
        "game data unavailable: this server's content isn't verified (see profile/engine settings)"
    );
}

#[test]
fn new_supply_v2_is_not_a_rustyscript_json_callback() {
    let value = probe_v2(
        r#"
export const apiVersion = 2;
        export function tick(api) {
  const typed = globalThis.__rs2b0t_supply_v2('foodCount', { items: [], foodName: 'Shark' });
  let jsonStyle = null;
  try {
    jsonStyle = globalThis.rustyscript.functions.__rs2b0t_supply_v2({
      op: 'foodCount',
      input: { items: [], foodName: 'Shark' },
    });
  } catch (e) {
    jsonStyle = { ok: false, error: String(e && (e.message || e)) };
  }
  globalThis.__probe = JSON.stringify({
    food: typeof api.foodCount,
    typedOk: typed && typed.ok === true,
    jsonStyleOk: jsonStyle && jsonStyle.ok === true,
    bindingsHasJsonRegister: false,
  });
}
"#,
        ClientRevision::R274,
    );
    assert_eq!(value["food"], "function");
    assert_eq!(value["typedOk"], true, "{value:?}");
    assert_eq!(value["jsonStyleOk"], false, "{value:?}");
}

#[test]
fn v1_food_count_preserves_falsy_skip_and_empty_name_match() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let value = probe_compat(
            r#"
import { foodCount } from '../../api/combat/food.js';
export default class T extends LoopingBot {
    loop() {
        const boom = {
            toString() { throw new Error('foodName boom'); },
            valueOf() { throw new Error('foodName boom'); },
        };
        let emptyBoom = 'threw';
        let namedBoom = '';
        try { emptyBoom = foodCount([], boom); } catch (e) { emptyBoom = String(e.message || e); }
        try { foodCount([{ name: null }], boom); } catch (e) { namedBoom = String(e.message || e); }
        globalThis.__probe = JSON.stringify({
            oddEmpty: foodCount([true, 1, 'x', false, 0, '', null, undefined], ''),
            oddShark: foodCount([true, 1, 'x', false, 0, '', null, undefined], 'Shark'),
            undefName: foodCount([{ name: undefined }, { name: null }, {}], ''),
            numberNameEmpty: foodCount([{ name: 0 }], ''),
            numberNameZero: foodCount([{ name: 0 }], 0),
            emptyBoom,
            namedBoom,
        });
    }
}
"#,
            revision,
        );
        assert_eq!(value["oddEmpty"], 3, "{value:?}");
        assert_eq!(value["oddShark"], 0, "{value:?}");
        assert_eq!(value["undefName"], 3, "{value:?}");
        assert_eq!(value["numberNameEmpty"], 0, "{value:?}");
        assert_eq!(value["numberNameZero"], 1, "{value:?}");
        assert_eq!(value["emptyBoom"], 0, "{value:?}");
        assert!(
            value["namedBoom"].as_str().unwrap_or("").contains("boom"),
            "{value:?}"
        );
    }
}

#[test]
fn v2_snapshot_inv_and_invalid_optional_types() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let data = api::game_data::for_revision(revision).unwrap();
        let iso = LoadIsolate::spawn_with_game_data(
            r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__probe = JSON.stringify({
    fromInv: api.foodCount({ items: api.snapshot.inv, foodName: 'Shark' }),
    arrayLike: api.foodCount({ items: { length: 1, 0: { name: 'Shark' } }, foodName: 'Shark' }),
    styleNum: api.combatKeepNames({ food: 'Lobster', style: 1 }),
    extraNum: api.combatKeepNames({ food: 'Lobster', extra: ['Coins', 1] }),
    wieldedNum: api.runesPerCast({ spellName: 'Wind Strike', wielded: ['Staff of air', 1] }),
    keepFoodNum: api.combatKeepNames({ food: 1 }),
    runesMissingWielded: api.runesPerCast({ spellName: 'Wind Strike' }),
  });
}
"#
            .into(),
            LoadShape::NativeTick,
            vec![],
            data,
        )
        .unwrap();
        let inv = [
            item("Shark", 385, 0),
            item("Shark", 385, 1),
            item("Coins", 995, 2),
        ];
        post_inv(&iso, 1, &inv);
        iso.on_game_tick(1);
        let value: serde_json::Value =
            serde_json::from_str(iso.probe("globalThis.__probe").unwrap().as_str().unwrap())
                .unwrap();
        iso.join();
        assert_eq!(value["fromInv"]["ok"], true, "{value:?}");
        assert_eq!(value["fromInv"]["value"], 2, "{value:?}");
        assert_eq!(value["arrayLike"]["error"], "invalid-args", "{value:?}");
        assert_eq!(value["styleNum"]["error"], "invalid-args", "{value:?}");
        assert_eq!(value["extraNum"]["error"], "invalid-args", "{value:?}");
        assert_eq!(value["wieldedNum"]["error"], "invalid-args", "{value:?}");
        assert_eq!(value["keepFoodNum"]["error"], "invalid-args", "{value:?}");
        assert_eq!(
            value["runesMissingWielded"]["error"], "invalid-args",
            "{value:?}"
        );
    }
}

#[test]
fn example_supply_helpers_v2_ts_runs_all_five_on_snapshot() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("supply_helpers_v2.ts");
    let src = std::fs::read_to_string(&path).expect("example source");
    let js = script::transpile_ts(&src).expect("transpile supply_helpers_v2.ts");
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let data = api::game_data::for_revision(revision).unwrap();
        let iso =
            LoadIsolate::spawn_with_game_data(js.clone(), LoadShape::NativeTick, vec![], data)
                .unwrap();
        let inv = [
            item("Shark", 385, 0),
            item("Shark", 385, 1),
            item("Trout", 333, 2),
        ];
        post_inv(&iso, 1, &inv);
        iso.on_game_tick(1);
        let err = iso
            .probe("globalThis.__rs2b0t_host.lastError || ''")
            .unwrap();
        let logs = iso.drain_logs();
        iso.join();
        assert_eq!(
            err.as_str().unwrap_or(""),
            "",
            "example lastError: {err:?} logs={logs:?}"
        );
        let last = logs
            .iter()
            .rev()
            .find(|line| line.contains("sharkSlots"))
            .unwrap_or_else(|| panic!("example logged helper results; logs={logs:?}"));
        let row: serde_json::Value = serde_json::from_str(last).unwrap();
        assert_eq!(row["sharkSlots"]["ok"], true, "{row:?}");
        assert_eq!(row["sharkSlots"]["value"], 2, "{row:?}");
        assert_eq!(row["breadHeal"]["ok"], true, "{row:?}");
        assert_eq!(row["breadHeal"]["value"], 4, "{row:?}");
        assert_eq!(
            row["keep"]["value"],
            serde_json::json!(["lobster", "Mind rune", "Air rune"]),
            "{row:?}"
        );
        assert_eq!(
            row["runes"]["value"],
            serde_json::json!([{ "rune": "Mind rune", "count": 1 }]),
            "{row:?}"
        );
        assert_eq!(
            row["escape"]["value"]["label"], "Varrock teleport",
            "{row:?}"
        );
        assert_eq!(row["escape"]["value"]["level"], 25, "{row:?}");
    }
}
