use client::io::ClientRevision;
use script::isolate_fb::{ItemRowInput, SnapshotInput, StatInput};
use script::{LoadIsolate, LoadShape};

fn item(name: &'static str, id: i32, slot: i32, count: i32) -> ItemRowInput<'static> {
    ItemRowInput {
        name: Some(name),
        count,
        id,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: -1,
        slot,
    }
}

fn stat(index: i32, name: &'static str, base: i32, effective: i32) -> StatInput<'static> {
    StatInput {
        index,
        name,
        xp: 0,
        base,
        effective,
    }
}

fn snapshot<'a>(
    tick: u64,
    inv: &'a [ItemRowInput<'a>],
    stats: &'a [StatInput<'a>],
) -> SnapshotInput<'a> {
    let mut input = SnapshotInput {
        tick,
        here: None,
        ingame: true,
        inv,
        inv_size: 28,
        stats,
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
    input
}

fn post_base(iso: &LoadIsolate, tick: u64) {
    iso.post_snapshot(script::isolate_fb::encode_snapshot(&snapshot(
        tick,
        &[],
        &[],
    )));
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

#[test]
fn v2_food_of_uses_fixed_heals_and_requires_selected_cache() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let value = probe_v2(
            r#"
export const apiVersion = 2;
export function tick(api) {
  const carry = [
    { item: 'Lobster', qty: 4 },
    { item: 'Prayer potion(4)', qty: 1 },
    { item: 'Cabbage', qty: 2 },
  ];
  globalThis.__probe = JSON.stringify({
    lobster: api.foodOf({ loadout: { carry }, fallback: 'Trout' }),
    cabbage: api.foodOf({ loadout: { carry: [{ item: 'Cabbage' }] }, fallback: 'Trout' }),
    empty: api.foodOf({ loadout: null, fallback: 'Trout' }),
    missingLoadout: api.foodOf({ fallback: 'Trout' }),
    badFallback: api.foodOf({ loadout: null, fallback: 1 }),
    badCarry: api.foodOf({ loadout: { carry: [{ item: 9 }] }, fallback: 'Trout' }),
  });
}
"#,
            revision,
        );
        assert_eq!(value["lobster"]["value"], "Lobster", "{value:?}");
        assert_eq!(value["cabbage"]["value"], "Trout", "{value:?}");
        assert_eq!(value["empty"]["value"], "Trout", "{value:?}");
        assert_eq!(value["missingLoadout"]["error"], "invalid-args");
        assert_eq!(value["badFallback"]["error"], "invalid-args");
        assert_eq!(value["badCarry"]["error"], "invalid-args");
    }
    let iso = LoadIsolate::spawn(
        r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__probe = JSON.stringify(api.foodOf({ loadout: { carry: [{ item: 'Lobster' }] }, fallback: 'Trout' }));
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
    assert_eq!(value["error"], "missing-selected-data");
}

#[test]
fn v2_gear_weapon_supplies_use_hat_first_slots_and_qty_limits() {
    let value = probe_v2(
        r#"
export const apiVersion = 2;
export function tick(api) {
  const loadout = {
    worn: {
      torso: 'Rune chainbody',
      righthand: 'Rune scimitar',
      hat: 'Rune full helm',
      mystery: 'Ignored slot',
    },
    unassigned: ['old helm', ''],
    carry: [{ item: 'Coins' }, { item: 'Lobster', qty: 10 }],
  };
  globalThis.__probe = JSON.stringify({
    gear: api.gearOf({ loadout }),
    empty: api.gearOf({ loadout: null }),
    badWorn: api.gearOf({ loadout: { worn: { hat: 1 } } }),
    weapon: api.weaponOf({ loadout, fallback: 'Bronze sword' }),
    blank: api.weaponOf({ loadout: { worn: { righthand: '  ' } }, fallback: 'Bronze sword' }),
    supplies: api.suppliesOf({ loadout }),
    qtyZero: api.suppliesOf({ loadout: { carry: [{ item: 'Coins', qty: 0 }] } }),
    qtyFrac: api.suppliesOf({ loadout: { carry: [{ item: 'Coins', qty: 1.5 }] } }),
    qtyOver: api.suppliesOf({ loadout: { carry: [{ item: 'Coins', qty: 4294967296 }] } }),
  });
}
"#,
        ClientRevision::R274,
    );
    assert_eq!(
        value["gear"]["value"],
        serde_json::json!([
            "Rune full helm",
            "Rune scimitar",
            "Rune chainbody",
            "old helm"
        ])
    );
    assert_eq!(value["empty"]["value"], serde_json::json!([]));
    assert_eq!(value["badWorn"]["error"], "invalid-args");
    assert_eq!(value["weapon"]["value"], "Rune scimitar");
    assert_eq!(value["blank"]["value"], "Bronze sword");
    assert_eq!(
        value["supplies"]["value"],
        serde_json::json!([{ "item": "Coins", "qty": 1 }, { "item": "Lobster", "qty": 10 }])
    );
    assert_eq!(value["qtyZero"]["error"], "invalid-args");
    assert_eq!(value["qtyFrac"]["error"], "invalid-args");
    assert_eq!(value["qtyOver"]["error"], "invalid-args");
}

#[test]
fn v2_range_loadout_uses_selected_darts_on_both_revisions() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let value = probe_v2(
            r#"
export const apiVersion = 2;
export function tick(api) {
  const names = ['Bronze dart','Iron dart','Steel dart','Black dart','Mithril dart','Adamant dart','Rune dart'];
  const darts = {};
  for (const name of names) {
    darts[name] = api.rangeLoadoutOf({ weapon: name, ammo: 'Iron arrow' });
  }
  globalThis.__probe = JSON.stringify({
    empty: api.rangeLoadoutOf({ weapon: '', ammo: 'Iron arrow' }),
    bow: api.rangeLoadoutOf({ weapon: 'Maple shortbow', ammo: 'Iron arrow' }),
    dart: api.rangeLoadoutOf({ weapon: 'Bronze dart', ammo: 'Iron arrow' }),
    dragon: api.rangeLoadoutOf({ weapon: 'Dragon dart', ammo: 'Iron arrow' }),
    bad: api.rangeLoadoutOf({ weapon: 1, ammo: 'Iron arrow' }),
    darts,
  });
}
"#,
            revision,
        );
        assert_eq!(
            value["empty"]["value"],
            serde_json::json!({"weapon": "", "projectile": "Iron arrow", "thrown": false})
        );
        assert_eq!(
            value["bow"]["value"],
            serde_json::json!({"weapon": "Maple shortbow", "projectile": "Iron arrow", "thrown": false})
        );
        assert_eq!(
            value["dart"]["value"],
            serde_json::json!({"weapon": "Bronze dart", "projectile": "Bronze dart", "thrown": true})
        );
        assert_eq!(
            value["dragon"]["value"],
            serde_json::json!({"weapon": "Dragon dart", "projectile": "Iron arrow", "thrown": false})
        );
        assert_eq!(value["bad"]["error"], "invalid-args");
        for name in [
            "Bronze dart",
            "Iron dart",
            "Steel dart",
            "Black dart",
            "Mithril dart",
            "Adamant dart",
            "Rune dart",
        ] {
            assert_eq!(
                value["darts"][name]["value"]["thrown"], true,
                "{name} {value:?}"
            );
            assert_eq!(value["darts"][name]["value"]["weapon"], name);
            assert_eq!(value["darts"][name]["value"]["projectile"], name);
        }
    }
    let iso = LoadIsolate::spawn(
        r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__probe = JSON.stringify(api.rangeLoadoutOf({ weapon: 'Bronze dart', ammo: 'Iron arrow' }));
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
    assert_eq!(value["error"], "missing-selected-data");
}

#[test]
fn v2_boost_and_plans_and_sip_keep_missingness_and_numeric_rules() {
    let value = probe_v2(
        r#"
export const apiVersion = 2;
export function tick(api) {
  const plans = api.plannedPotions({
    carry: [
      { item: 'Super attack(1)', qty: 7 },
      { item: 'Lobster', qty: 3 },
      { item: 'Prayer potion(4)', qty: 1 },
    ],
  });
  const emptyCarry = api.plannedPotions({ carry: [] });
  const pair = plans.value;
  globalThis.__probe = JSON.stringify({
    faded: {
      unboosted: api.boostFaded({ base: 70, effective: 70 }),
      onTenth: api.boostFaded({ base: 70, effective: 77 }),
      above: api.boostFaded({ base: 70, effective: 78 }),
      drained: api.boostFaded({ base: 70, effective: 60 }),
      unread: api.boostFaded({ base: 0, effective: 0 }),
      negative: api.boostFaded({ base: -10, effective: -10 }),
      nan: api.boostFaded({ base: Number.NaN, effective: 70 }),
    },
    planned: plans,
    emptyCarry,
    qtyRequired: api.plannedPotions({ carry: [{ item: 'Super attack(4)' }] }),
    sipDue: api.potionToSip({
      plans: pair,
      held: [1, 1],
      levels: [
        { skill: 'attack', base: 70, effective: 70 },
        { skill: 'strength', base: 70, effective: 85 },
      ],
    }),
    sipEmpty: api.potionToSip({ plans: [], held: [], levels: [] }),
    sipSkipHeld: api.potionToSip({
      plans: pair,
      held: [0, 1],
      levels: [{ skill: 'strength', base: 70, effective: 70 }],
    }),
    sipMissing: api.potionToSip({
      plans: pair,
      held: [1, 1],
      levels: [{ skill: 'strength', base: 70, effective: 70 }],
    }),
    sipZeroKnown: api.potionToSip({
      plans: pair,
      held: [1, 1],
      levels: [
        { skill: 'ATTACK', base: 0, effective: 0 },
        { skill: 'strength', base: 70, effective: 70 },
      ],
    }),
    sipDup: api.potionToSip({
      plans: pair,
      held: [1, 1],
      levels: [
        { skill: 'attack', base: 70, effective: 70 },
        { skill: 'Attack', base: 70, effective: 70 },
      ],
    }),
    sipLen: api.potionToSip({ plans: pair, held: [1], levels: [] }),
    sipHeldNeg: api.potionToSip({ plans: pair, held: [-1, 0], levels: [] }),
  });
}
"#,
        ClientRevision::R274,
    );
    assert_eq!(value["faded"]["unboosted"]["value"], true);
    assert_eq!(value["faded"]["onTenth"]["value"], true);
    assert_eq!(value["faded"]["above"]["value"], false);
    assert_eq!(value["faded"]["drained"]["value"], false);
    assert_eq!(value["faded"]["unread"]["value"], false);
    assert_eq!(value["faded"]["negative"]["ok"], true);
    assert_eq!(value["faded"]["nan"]["error"], "invalid-args");
    assert_eq!(value["planned"]["value"][0]["flask"], "Super attack(1)");
    assert_eq!(value["planned"]["value"][0]["want"], 7);
    assert_eq!(value["planned"]["value"][0]["short"], "Att");
    assert_eq!(value["planned"]["value"][1]["flask"], "Super strength(3)");
    assert_eq!(value["planned"]["value"][1]["want"], 1);
    assert_eq!(value["emptyCarry"]["value"][0]["flask"], "Super attack(3)");
    assert_eq!(value["qtyRequired"]["error"], "invalid-args");
    assert_eq!(value["sipDue"]["value"]["skill"], "attack");
    assert!(value["sipEmpty"]["value"].is_null());
    assert_eq!(value["sipSkipHeld"]["value"]["skill"], "strength");
    assert_eq!(value["sipMissing"]["error"], "missing-observation");
    assert_eq!(value["sipZeroKnown"]["value"]["skill"], "strength");
    assert_eq!(value["sipDup"]["error"], "invalid-args");
    assert_eq!(value["sipLen"]["error"], "invalid-args");
    assert_eq!(value["sipHeldNeg"]["error"], "invalid-args");
}

#[test]
fn new_loadout_v2_is_not_a_rustyscript_json_callback() {
    let value = probe_v2(
        r#"
export const apiVersion = 2;
export function tick(api) {
  const typed = globalThis.__rs2b0t_loadout_v2('boostFaded', { base: 70, effective: 70 });
  let jsonStyle = null;
  try {
    jsonStyle = globalThis.rustyscript.functions.__rs2b0t_loadout_v2({
      op: 'boostFaded',
      input: { base: 70, effective: 70 },
    });
  } catch (e) {
    jsonStyle = { ok: false, error: String(e && (e.message || e)) };
  }
  globalThis.__probe = JSON.stringify({
    food: typeof api.foodOf,
    typedOk: typed && typed.ok === true,
    jsonStyleOk: jsonStyle && jsonStyle.ok === true,
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
fn example_loadout_potion_v2_ts_runs_all_eight_on_real_inv_stats() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("loadout_potion_v2.ts");
    let src = std::fs::read_to_string(&path).expect("example source");
    let js = script::transpile_ts(&src).expect("transpile loadout_potion_v2.ts");
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let data = api::game_data::for_revision(revision).unwrap();
        let iso =
            LoadIsolate::spawn_with_game_data(js.clone(), LoadShape::NativeTick, vec![], data)
                .unwrap();
        let inv = [item("Super attack(4)", 2436, 0, 1)];
        let stats = [stat(0, "Attack", 70, 70), stat(2, "Strength", 70, 85)];
        iso.post_snapshot(script::isolate_fb::encode_snapshot(&snapshot(
            1, &inv, &stats,
        )));
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
            .find(|line| line.contains("\"food\""))
            .unwrap_or_else(|| panic!("example logged helper results; logs={logs:?}"));
        let row: serde_json::Value = serde_json::from_str(last).unwrap();
        assert_eq!(row["food"]["value"], "Lobster", "{row:?}");
        assert_eq!(
            row["gear"]["value"],
            serde_json::json!([
                "Rune full helm",
                "Rune scimitar",
                "Rune chainbody",
                "old helm"
            ]),
            "{row:?}"
        );
        assert_eq!(row["supplies"]["value"][0]["item"], "Lobster", "{row:?}");
        assert_eq!(row["weapon"]["value"], "Rune scimitar", "{row:?}");
        assert_eq!(row["range"]["value"]["thrown"], true, "{row:?}");
        assert_eq!(row["faded"]["value"], true, "{row:?}");
        assert_eq!(
            row["planned"]["value"][0]["flask"], "Super attack(4)",
            "{row:?}"
        );
        assert_eq!(row["sip"]["value"]["skill"], "attack", "{row:?}");
    }
}
