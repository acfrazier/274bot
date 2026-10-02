//! The V8 boundary of the gather helpers over the typed gathering catalog: argument gates, helper-result shapes and
//! the values that cross into JS. Catalog and adapter behavior is tested in `crates/api/tests/gather_*.rs`.
use client::io::ClientRevision;
use script::{LoadIsolate, LoadShape};

fn post_base(iso: &LoadIsolate, tick: u64) {
    let input = script::isolate_fb::SnapshotInput {
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
        self_target_kind: 0,
        self_target_index: -1,
        widgets: &[],
        user_move_intent_seq: 0,
        walk_outcome_cancel_reason: Default::default(),
    };
    iso.post_snapshot(script::isolate_fb::encode_snapshot(&input));
}

fn probe(src: &str, revision: ClientRevision) -> serde_json::Value {
    let data = api::game_data::for_revision(revision).unwrap();
    let iso =
        LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data).unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let value = iso.probe("globalThis.__probe").unwrap();
    iso.join();
    serde_json::from_str(value.as_str().unwrap()).unwrap()
}

/// Selected data that never bound a pin (no engine/content commits), so no gathering family can be admitted.
fn data_without_family() -> std::sync::Arc<api::game_data::SelectedGameData> {
    let raw = r#"{
        "schema_version": 4,
        "revision": 274,
        "provenance": {
            "cache_identity": {"cache_id": "x"},
            "inputs": [],
            "content_inputs": [],
            "decoder_sources": []
        },
        "items": [],
        "consumption": [],
        "pickpocket": []
    }"#;
    std::sync::Arc::new(serde_json::from_str(raw).expect("selected data without a pin"))
}

fn catalog(revision: ClientRevision) -> std::sync::Arc<api::gather_methods::GatherCatalog> {
    let data = api::game_data::for_revision(revision).unwrap();
    api::selected::FamilyPreparation::run(move |worker| data.prepare_gathering(worker))
        .unwrap()
        .join()
        .unwrap()
        .expect("gathering family prepares")
}

fn helper_ok(value: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "ok": true, "value": value })
}

#[test]
fn v2_methods_are_sync_helper_results_with_locked_errors() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const omitted = api.gatherMethods();
  const iron = api.gatherResource({ name: 'Iron' });
  globalThis.__probe = JSON.stringify({
    omitted,
    empty: api.gatherMethods({}),
    woodcutting: api.gatherMethods({ skill: ' Woodcutting ' }),
    iron,
    trout: api.gatherResource({ name: 'raw_trout' }),
    ironThen: typeof iron.then,
    nullArg: api.gatherMethods(null),
    stringArg: api.gatherMethods('woodcutting'),
    nullSkill: api.gatherMethods({ skill: null }),
    blankSkill: api.gatherMethods({ skill: '   ' }),
    woods: api.gatherMethods({ skill: 'woods' }),
    nullName: api.gatherResource({ name: null }),
    missingName: api.gatherResource({}),
    rocks: api.gatherResource({ name: 'Rocks' }),
    blankName: api.gatherResource({ name: '   ' }),
    promise: omitted instanceof Promise,
  });
}
"#;
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let value = probe(src, revision);
        let catalog = catalog(revision);
        let methods = |skill| api::gather_methods::gather_methods(Some(&catalog), skill).unwrap();
        let resource = |name| api::gather_methods::gather_resource(Some(&catalog), name).unwrap();
        // Whole results cross into JS without losing a null, a number or an unknown/partial marker.
        assert_eq!(value["omitted"], helper_ok(methods(None)), "{revision:?}");
        assert_eq!(value["empty"], value["omitted"]);
        assert_eq!(
            value["woodcutting"],
            helper_ok(methods(Some("woodcutting")))
        );
        assert_eq!(value["iron"], helper_ok(resource("Iron")));
        assert_eq!(value["trout"], helper_ok(resource("raw_trout")));
        assert_eq!(value["ironThen"], "undefined");
        assert_eq!(value["nullArg"]["error"], "invalid-args");
        assert_eq!(value["stringArg"]["error"], "invalid-args");
        assert_eq!(value["nullSkill"]["error"], "invalid-args");
        assert_eq!(value["blankSkill"]["error"], "unknown-skill");
        assert_eq!(value["woods"]["error"], "unknown-skill");
        assert_eq!(value["nullName"]["error"], "invalid-args");
        assert_eq!(value["missingName"]["error"], "invalid-args");
        assert_eq!(value["rocks"]["error"], "unknown-resource");
        assert_eq!(value["blankName"]["error"], "unknown-resource");
        assert_eq!(value["promise"], false);
    }
}

#[test]
fn v2_invalid_args_before_missing_slot_and_missing_family() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__probe = JSON.stringify({
    nullArg: api.gatherMethods(null),
    numberSkill: api.gatherMethods({ skill: 1 }),
    numberName: api.gatherResource({ name: 1 }),
    omitted: api.gatherMethods(),
    empty: api.gatherMethods({}),
    badSkill: api.gatherMethods({ skill: 'wc' }),
    missName: api.gatherResource({ name: 'nope' }),
    iron: api.gatherResource({ name: 'iron' }),
  });
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let empty: serde_json::Value =
        serde_json::from_str(iso.probe("globalThis.__probe").unwrap().as_str().unwrap()).unwrap();
    iso.join();
    assert_eq!(empty["nullArg"]["error"], "invalid-args");
    assert_eq!(empty["numberSkill"]["error"], "invalid-args");
    assert_eq!(empty["numberName"]["error"], "invalid-args");
    for key in ["omitted", "empty", "badSkill", "missName"] {
        assert_eq!(
            empty[key]["error"],
            "game data unavailable: this server's content isn't verified (see profile/engine settings)",
            "{key} {empty:?}"
        );
    }

    let iso = LoadIsolate::spawn_with_game_data(
        src.into(),
        LoadShape::NativeTick,
        vec![],
        data_without_family(),
    )
    .unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let missing: serde_json::Value =
        serde_json::from_str(iso.probe("globalThis.__probe").unwrap().as_str().unwrap()).unwrap();
    iso.join();
    assert_eq!(missing["numberSkill"]["error"], "invalid-args");
    assert_eq!(missing["numberName"]["error"], "invalid-args");
    assert_eq!(
        missing["omitted"]["error"],
        "family-unavailable:gather_methods"
    );
    assert_eq!(
        missing["badSkill"]["error"],
        "family-unavailable:gather_methods"
    );
    assert_eq!(
        missing["missName"]["error"],
        "family-unavailable:gather_methods"
    );
    assert_eq!(
        missing["iron"]["error"],
        "family-unavailable:gather_methods"
    );
}

#[test]
fn v2_install_is_typed_not_a_json_op_or_interact() {
    let value = probe(
        r#"
export const apiVersion = 2;
export function tick(api) {
  const before = (api.snapshot && 0) || 0;
  const row = api.gatherResource({ name: 'iron' });
  let requested = null;
  try { api.request({ op: 'gatherMethods' }); requested = 'ok'; }
  catch (e) { requested = String(e && (e.message || e)); }
  globalThis.__probe = JSON.stringify({
    rowOk: row.ok === true,
    interact: globalThis.__rs2b0t_host && globalThis.__rs2b0t_host.interact,
    requested,
    before,
  });
}
"#,
        ClientRevision::R274,
    );
    assert_eq!(value["rowOk"], true, "{value:?}");
    assert!(
        value["interact"].is_null()
            || value["interact"]
                .as_array()
                .is_some_and(|rows| rows.is_empty()),
        "{value:?}"
    );
    assert!(
        value["requested"]
            .as_str()
            .unwrap_or("")
            .contains("not impl"),
        "{value:?}"
    );
}

#[test]
fn v2_placements_absence_is_family_unavailable_not_empty_rows() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const box = { min_x: 2355, min_z: 3412, max_x: 2356, max_z: 3425, level: 0 };
  globalThis.__probe = JSON.stringify({
    oak: api.gatherPlacements({ resource: 'oak', region: box, limit: 64 }),
    iron: api.gatherPlacements({ resource: 'iron', region: box, limit: 64 }),
  });
}
"#;
    // Selected data with no verified pin cannot admit the gathering family: absence is a token, not empty rows.
    let iso = LoadIsolate::spawn_with_game_data(
        src.into(),
        LoadShape::NativeTick,
        vec![],
        data_without_family(),
    )
    .unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let missing: serde_json::Value =
        serde_json::from_str(iso.probe("globalThis.__probe").unwrap().as_str().unwrap()).unwrap();
    iso.join();
    assert_eq!(
        missing["oak"]["error"], "family-unavailable:gather_placements",
        "{missing:?}"
    );
    assert_eq!(missing["iron"]["error"], missing["oak"]["error"]);
    assert!(missing["oak"].get("value").is_none(), "{missing:?}");
    assert_ne!(missing["oak"]["error"], "family-unavailable:gather_methods");
}

#[test]
fn v2_placements_args_precede_the_selected_pin() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const box = { min_x: 1, min_z: 1, max_x: 2, max_z: 2, level: 0 };
  globalThis.__probe = JSON.stringify({
    omitted: api.gatherPlacements(),
    nullArg: api.gatherPlacements(null),
    arrayArg: api.gatherPlacements([]),
    stringArg: api.gatherPlacements('oak'),
    omittedRegion: api.gatherPlacements({ resource: 'oak', limit: 64 }),
    omittedRegionBadLimit: api.gatherPlacements({ resource: 'oak', limit: 65 }),
    nullRegion: api.gatherPlacements({ resource: 'oak', region: null, limit: 64 }),
    shortRegion: api.gatherPlacements({ resource: 'oak', region: { min_x: 1, min_z: 1, max_x: 2, max_z: 2 }, limit: 64 }),
    planeRegion: api.gatherPlacements({ resource: 'oak', region: { min_x: 1, min_z: 1, max_x: 2, max_z: 2, plane: 0 }, limit: 64 }),
    radiusRegion: api.gatherPlacements({ resource: 'oak', region: { cx: 1, cz: 1, plane: 0, radius: 4 }, limit: 64 }),
    numberResource: api.gatherPlacements({ resource: 1281, region: box, limit: 64 }),
    omittedResource: api.gatherPlacements({ region: box, limit: 64 }),
    limit65: api.gatherPlacements({ resource: 'oak', region: box, limit: 65 }),
    limit0: api.gatherPlacements({ resource: 'oak', region: box, limit: 0 }),
    limitFraction: api.gatherPlacements({ resource: 'oak', region: box, limit: 1.5 }),
    limitOmitted: api.gatherPlacements({ resource: 'oak', region: box }),
    levelOmitted: api.gatherPlacements({ resource: 'oak', region: { min_x: 1, min_z: 1, max_x: 2, max_z: 2 }, limit: 64 }),
    numberAfter: api.gatherPlacements({ resource: 'oak', region: box, limit: 64, after: 5 }),
    nullAfter: api.gatherPlacements({ resource: 'oak', region: box, limit: 64, after: null }),
    good: api.gatherPlacements({ resource: 'oak', region: box, limit: 64 }),
  });
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let value: serde_json::Value =
        serde_json::from_str(iso.probe("globalThis.__probe").unwrap().as_str().unwrap()).unwrap();
    iso.join();
    for key in [
        "omitted",
        "nullArg",
        "arrayArg",
        "stringArg",
        "nullRegion",
        "shortRegion",
        "planeRegion",
        "radiusRegion",
        "numberResource",
        "omittedResource",
        "limit65",
        "limit0",
        "limitFraction",
        "limitOmitted",
        "levelOmitted",
        "numberAfter",
        "nullAfter",
    ] {
        assert_eq!(value[key]["error"], "invalid-args", "{key}: {value:?}");
    }
    // A present object without the region key is missing-region, before the
    // resource, the limit, and the region shape.
    assert_eq!(
        value["omittedRegion"]["error"], "missing-region",
        "{value:?}"
    );
    assert_eq!(
        value["omittedRegionBadLimit"]["error"], "missing-region",
        "{value:?}"
    );
    // Args first: a well-formed call is the only one that reaches the pin.
    assert_eq!(
        value["good"]["error"],
        "game data unavailable: this server's content isn't verified (see profile/engine settings)",
        "{value:?}"
    );
}

#[test]
fn v2_placements_are_sync_helper_results_with_locked_errors() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const box = { min_x: 2355, min_z: 3412, max_x: 2356, max_z: 3425, level: 0 };
  const wide = { min_x: 0, min_z: 0, max_x: 99999, max_z: 99999, level: 0 };
  const oak = api.gatherPlacements({ resource: ' Oak ', region: box, limit: 64 });
  const first = api.gatherPlacements({ resource: 'normal', region: wide, limit: 2 });
  globalThis.__probe = JSON.stringify({
    oak,
    oakThen: typeof oak.then,
    miss: api.gatherPlacements({ resource: 'oak', region: { min_x: 3000, min_z: 3000, max_x: 3001, max_z: 3001, level: 0 }, limit: 64 }),
    iron: api.gatherPlacements({ resource: 'iron', region: box, limit: 64 }),
    jungle: api.gatherPlacements({ resource: 'jungle', region: box, limit: 64 }),
    nope: api.gatherPlacements({ resource: 'nope', region: box, limit: 64 }),
    first,
    second: api.gatherPlacements({ resource: 'normal', region: wide, limit: 2, after: first.value.next }),
    badCursor: api.gatherPlacements({ resource: 'normal', region: wide, limit: 2, after: 'not-a-spot' }),
    promise: oak instanceof Promise,
  });
}
"#;
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let value = probe(src, revision);
        let catalog = catalog(revision);
        let placements = |resource, region: [i32; 5], after: Option<&str>, limit| {
            let region = api::gather_methods::SceneRegionInput {
                min_x: region[0],
                min_z: region[1],
                max_x: region[2],
                max_z: region[3],
                level: region[4],
            };
            api::gather_methods::gather_placements(Some(&catalog), resource, &region, after, limit)
        };
        let (box_, wide) = ([2355, 3412, 2356, 3425, 0], [0, 0, 99999, 99999, 0]);
        // Rows, nulls and cursors cross into JS exactly as the adapter produced them.
        let oak = placements(" Oak ", box_, None, 64).unwrap();
        assert_eq!(value["oak"], helper_ok(oak.clone()), "{revision:?}");
        assert_eq!(oak["rows"].as_array().map(Vec::len), Some(2));
        assert_eq!(oak["qualification"], "complete");
        assert_eq!(value["oakThen"], "undefined");
        assert_eq!(
            value["miss"],
            helper_ok(placements("oak", [3000, 3000, 3001, 3001, 0], None, 64).unwrap())
        );
        assert_eq!(
            value["iron"],
            helper_ok(placements("iron", box_, None, 64).unwrap())
        );
        let first = placements("normal", wide, None, 2).unwrap();
        assert_eq!(first["truncated"], true);
        assert_eq!(value["first"], helper_ok(first.clone()));
        let after = first["next"].as_str().unwrap();
        assert_eq!(
            value["second"],
            helper_ok(placements("normal", wide, Some(after), 2).unwrap())
        );
        assert_ne!(
            value["second"]["value"]["rows"], first["rows"],
            "the cursor moved on"
        );
        // Unknown coverage crosses as data, not as an error and not as an empty complete page.
        let jungle = placements("jungle", box_, None, 64).unwrap();
        assert_eq!(jungle["qualification"], "unknown");
        assert_eq!(value["jungle"], helper_ok(jungle));
        assert_eq!(value["nope"]["error"], "unknown-resource");
        assert_eq!(value["badCursor"]["error"], "invalid-args");
        assert_eq!(value["promise"], false, "{value:?}");
    }
}

#[test]
fn v2_placements_bridge_gates_its_own_args() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const bridge = (input) => globalThis.__rs2b0t_gather_methods_v2('gatherPlacements', input);
  const box = { min_x: 2355, min_z: 3412, max_x: 2356, max_z: 3425, level: 0 };
  globalThis.__probe = JSON.stringify({
    omittedRegion: bridge({ resource: 'oak', limit: 64 }),
    undefinedRegion: bridge({ resource: 'oak', region: undefined, limit: 64 }),
    numberRegion: bridge({ resource: 'oak', region: 1, limit: 64 }),
    arrayRegion: bridge({ resource: 'oak', region: [], limit: 64 }),
    stringLimit: bridge({ resource: 'oak', region: box, limit: '64' }),
    fractionLimit: bridge({ resource: 'oak', region: box, limit: 1.5 }),
    limit65: bridge({ resource: 'oak', region: box, limit: 65 }),
    fractionLevel: bridge({ resource: 'oak', region: { min_x: 2355, min_z: 3412, max_x: 2356, max_z: 3425, level: 0.5 }, limit: 64 }),
    numberResource: bridge({ resource: 1281, region: box, limit: 64 }),
    numberAfter: bridge({ resource: 'oak', region: box, limit: 64, after: 5 }),
    nullInput: bridge(null),
    good: bridge({ resource: 'oak', region: box, limit: 64 }),
  });
}
"#;
    let value = probe(src, ClientRevision::R274);
    assert_eq!(
        value["omittedRegion"]["error"], "missing-region",
        "{value:?}"
    );
    for key in [
        "undefinedRegion",
        "numberRegion",
        "arrayRegion",
        "stringLimit",
        "fractionLimit",
        "limit65",
        "fractionLevel",
        "numberResource",
        "nullInput",
        "numberAfter",
    ] {
        assert_eq!(value[key]["error"], "invalid-args", "{key}: {value:?}");
    }
    assert_eq!(value["good"]["ok"], true, "{value:?}");
    assert_eq!(
        value["good"]["value"]["rows"].as_array().map(Vec::len),
        Some(2),
        "{value:?}"
    );
}

#[test]
fn example_gather_methods_v2_is_one_read_only_fact_call() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("gather_methods_v2.ts");
    let src = std::fs::read_to_string(&path).expect("example source");
    assert!(!src.contains("request("));
    assert!(!src.contains("h.interact"));
    assert_eq!(src.matches("api.gather").count(), 1);
    let js = script::transpile_ts(&src).expect("transpile gather_methods_v2.ts");
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(js, LoadShape::NativeTick, vec![], data).unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let err = iso
        .probe("globalThis.__rs2b0t_host.lastError || ''")
        .unwrap();
    let logs = iso.drain_logs();
    iso.join();
    assert_eq!(err.as_str().unwrap_or(""), "", "example lastError: {err:?}");
    let last = logs
        .iter()
        .rev()
        .find(|line| line.contains("\"rows\""))
        .unwrap_or_else(|| panic!("example logged a fact call; logs={logs:?}"));
    let row: serde_json::Value = serde_json::from_str(last).unwrap();
    assert_eq!(row["ok"], true, "{row:?}");
    assert!(row["value"]["rows"]
        .as_array()
        .is_some_and(|rows| !rows.is_empty()));
}

#[test]
fn example_gather_placements_v2_is_one_read_only_fact_call() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("gather_placements_v2.ts");
    let src = std::fs::read_to_string(&path).expect("example source");
    assert!(!src.contains("request("));
    assert!(!src.contains("h.interact"));
    assert_eq!(src.matches("api.gatherPlacements").count(), 1);
    assert!(
        !path.with_extension("js").exists(),
        "the example is a .ts read; no compiled sibling"
    );
    let js = script::transpile_ts(&src).expect("transpile gather_placements_v2.ts");
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(js, LoadShape::NativeTick, vec![], data).unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let err = iso
        .probe("globalThis.__rs2b0t_host.lastError || ''")
        .unwrap();
    let logs = iso.drain_logs();
    iso.join();
    assert_eq!(err.as_str().unwrap_or(""), "", "example lastError: {err:?}");
    let last = logs
        .iter()
        .rev()
        .find(|line| line.contains("\"resource_ids\""))
        .unwrap_or_else(|| panic!("example logged a placements call; logs={logs:?}"));
    let row: serde_json::Value = serde_json::from_str(last).unwrap();
    assert_eq!(row["ok"], true, "{row:?}");
    assert_eq!(
        row["value"]["rows"].as_array().map(Vec::len),
        Some(2),
        "{row:?}"
    );
    assert_eq!(
        row["value"]["resource_ids"],
        serde_json::json!([{ "alias": "oaktree", "id": 1281 }]),
        "{row:?}"
    );
}

#[test]
fn frozen_gas_rock_set_skips_gas_variants_on_the_coal_query_path() {
    let src = r#"
import { GAS_ROCK_IDS as apiGas } from '@rs2b0t/api';
import { GAS_ROCK_IDS } from '../../data/miningRocks.js';
export default class T extends LoopingBot {
    loop() {
        const coalCandidates = [2125, 2096, 2126, 2097];
        globalThis.__probe = JSON.stringify({
            isSet: GAS_ROCK_IDS instanceof Set && apiGas instanceof Set,
            gasCoal: GAS_ROCK_IDS.has(2125) && apiGas.has(2126),
            realCoal: GAS_ROCK_IDS.has(2096) || apiGas.has(2097),
            mineable: coalCandidates.filter(id => !GAS_ROCK_IDS.has(id)),
        });
    }
}
"#;
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let data = api::game_data::for_revision(revision).unwrap();
        let iso =
            LoadIsolate::spawn_with_game_data(src.into(), LoadShape::CompatClass, vec![], data)
                .unwrap();
        post_base(&iso, 1);
        iso.on_game_tick(1);
        let value = iso.probe("globalThis.__probe").unwrap();
        iso.join();
        let value: serde_json::Value = serde_json::from_str(value.as_str().unwrap()).unwrap();
        assert_eq!(value["isSet"], true, "{revision:?}: {value:?}");
        assert_eq!(value["gasCoal"], true, "{revision:?}: {value:?}");
        assert_eq!(value["realCoal"], false, "{revision:?}: {value:?}");
        assert_eq!(
            value["mineable"],
            serde_json::json!([2096, 2097]),
            "{revision:?}: {value:?}"
        );
    }
}
