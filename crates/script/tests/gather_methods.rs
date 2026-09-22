//! Gather query over the landed schema-4 family. Not a tool table. Not a prayer machine.
use client::io::ClientRevision;
use script::{LoadIsolate, LoadShape};

fn facts(revision: ClientRevision) -> api::game_data::GatherMethodsFacts {
    api::game_data::for_revision(revision)
        .unwrap()
        .gather_methods()
        .expect("landed family")
        .clone()
}

fn methods(revision: ClientRevision, skill: Option<&str>) -> serde_json::Value {
    let facts = facts(revision);
    api::gather_methods::gather_methods(Some(&facts), skill).expect("methods")
}

fn resource(revision: ClientRevision, name: &str) -> Result<serde_json::Value, &'static str> {
    let facts = facts(revision);
    api::gather_methods::gather_resource(Some(&facts), name)
}

fn rows(value: &serde_json::Value) -> &Vec<serde_json::Value> {
    value["rows"].as_array().expect("rows")
}

fn skill_of(row: &serde_json::Value) -> &str {
    row["skill"].as_str().expect("skill")
}

fn key_of(row: &serde_json::Value) -> &str {
    row["resource_key"].as_str().expect("resource_key")
}

#[test]
fn missing_family_is_unavailable_before_skill_or_name() {
    let missing = api::gather_methods::gather_methods(None, Some("wc"));
    assert_eq!(missing, Err("family-unavailable:gather_methods"));
    let named = api::gather_methods::gather_resource(None, "not-a-resource");
    assert_eq!(named, Err("family-unavailable:gather_methods"));
    let would_hit = api::gather_methods::gather_resource(None, "iron");
    assert_eq!(would_hit, Err("family-unavailable:gather_methods"));
    assert_ne!(
        api::gather_methods::gather_methods(None, None),
        Ok(serde_json::json!({ "rows": [], "coverage": [] }))
    );
}

#[test]
fn omitted_skill_is_woods_then_mining_then_fishing_in_vector_order() {
    let woods = [
        "normal", "jungle", "burnt", "achey", "oak", "willow", "maple", "yew", "magic", "hollow",
    ];
    let mining = [
        "rune stones",
        "clay",
        "copper",
        "tin",
        "iron",
        "coal",
        "gold",
        "silver",
        "mithril",
        "adamantite",
        "runite",
        "blurite",
        "gems",
        "rock",
        "limestone",
        "limestone",
        "limestone",
    ];
    let fishing = [
        "category_453",
        "freshfish",
        "rarefish",
        "memberfish",
        "saltfish",
        "unknown",
        "category_632",
        "category_633",
        "slimeyfish",
    ];
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let value = methods(revision, None);
        let rows = rows(&value);
        assert_eq!(rows.len(), 36, "{revision:?}");
        assert!(value.get("display").is_none());
        let keys: Vec<&str> = rows[..10].iter().map(key_of).collect();
        assert_eq!(keys, woods, "{revision:?}");
        assert!(rows[..10].iter().all(|row| skill_of(row) == "woodcutting"));
        let mine: Vec<&str> = rows[10..27].iter().map(key_of).collect();
        assert_eq!(mine, mining, "{revision:?}");
        assert!(rows[10..27].iter().all(|row| skill_of(row) == "mining"));
        let fish: Vec<&str> = rows[27..]
            .iter()
            .map(|row| row["category"].as_str().unwrap())
            .collect();
        assert_eq!(fish, fishing, "{revision:?}");
        assert!(rows[27..].iter().all(|row| skill_of(row) == "fishing"));
        assert!(rows.iter().all(|row| row.get("display").is_none()));
    }
}

#[test]
fn skill_filter_keeps_bucket_order_and_does_not_filter_coverage() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let all = methods(revision, None);
        let wood = methods(revision, Some(" Woodcutting "));
        assert_eq!(rows(&wood).len(), 10);
        assert_eq!(
            rows(&wood).iter().map(key_of).collect::<Vec<_>>(),
            rows(&all).iter().take(10).map(key_of).collect::<Vec<_>>()
        );
        assert_eq!(wood["coverage"], all["coverage"]);
        let mine = methods(revision, Some("MINING"));
        assert_eq!(rows(&mine).len(), 17);
        assert!(rows(&mine).iter().all(|row| skill_of(row) == "mining"));
        assert_eq!(mine["coverage"], all["coverage"]);
        let fish = methods(revision, Some("fishing"));
        assert_eq!(rows(&fish).len(), 9);
        assert!(rows(&fish).iter().all(|row| {
            skill_of(row) == "fishing" && row.get("loc_ids").is_none() && row.get("tile").is_none()
        }));
        assert_eq!(fish["coverage"], all["coverage"]);
    }
    let facts = facts(ClientRevision::R274);
    for skill in ["", "   ", "woods", "wc", "mine", "fish", "unknown"] {
        assert_eq!(
            api::gather_methods::gather_methods(Some(&facts), Some(skill)),
            Err("unknown-skill"),
            "{skill:?}"
        );
    }
}

#[test]
fn resource_match_is_trimmed_key_only() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let iron = resource(revision, "  Iron  ").unwrap();
        assert!(iron.get("coverage").is_none());
        assert!(iron.get("display").is_none());
        let iron_rows = rows(&iron);
        assert_eq!(iron_rows.len(), 1);
        assert_eq!(key_of(&iron_rows[0]), "iron");
        assert_eq!(skill_of(&iron_rows[0]), "mining");
        assert_eq!(iron_rows[0]["publication"], serde_json::Value::Null);
        assert_eq!(
            iron_rows[0]["loc_ids"][0],
            serde_json::json!({ "alias": "ironrock1", "id": 2092 })
        );
        assert_eq!(
            iron_rows[0]["loc_ids"][1],
            serde_json::json!({ "alias": "ironrock2", "id": 2093 })
        );
        assert_eq!(
            iron_rows[0]["output"],
            serde_json::json!({ "alias": "iron_ore", "id": 440 })
        );
        assert_eq!(
            iron_rows[0]["missing_transform"],
            serde_json::json!(["macro_ironrock1", "macro_ironrock2"])
        );
        assert!(iron_rows[0]["loc_ids"][0].is_object());
        for miss in [
            "Iron ore",
            "iron_ore",
            "ironrock1",
            "ironrock2",
            "gem_rock",
            "logs",
            "runestones",
            "rune_stones",
            "blankrune",
            "Rocks",
            "Tree",
            "Fishing spot",
            "freshfish",
            "rarefish",
            "unknown",
            "dungeon_tree_closed",
            "karam_dungeon_exit",
            "   ",
            "",
        ] {
            assert_eq!(
                resource(revision, miss),
                Err("unknown-resource"),
                "{revision:?} {miss:?}"
            );
        }
        let stones = resource(revision, "Rune stones").unwrap();
        assert_eq!(rows(&stones).len(), 1);
        assert_eq!(key_of(&rows(&stones)[0]), "rune stones");
        let gems = resource(revision, "gems").unwrap();
        assert_eq!(key_of(&rows(&gems)[0]), "gems");
        assert_eq!(rows(&gems)[0]["table"], "gem_rock");
        let rock = resource(revision, "rock").unwrap();
        assert_eq!(rows(&rock)[0]["table"], "desertrescue_rock");
        assert_eq!(
            rows(&rock)[0]["loc_ids"][0],
            serde_json::json!({ "alias": "punishrocks", "id": 2704 })
        );
    }
}

#[test]
fn limestone_stays_three_rows_in_stored_order() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let value = resource(revision, "limestone").unwrap();
        let rows = rows(&value);
        assert_eq!(rows.len(), 3, "{revision:?}");
        let chain: Vec<(&str, i64, i64)> = rows
            .iter()
            .map(|row| {
                (
                    row["table"].as_str().unwrap(),
                    row["loc_ids"][0]["id"].as_i64().unwrap(),
                    row["empty_ids"][0]["id"].as_i64().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            chain,
            vec![
                ("limestone_rock3", 4027, 4028),
                ("limestone_rock2", 4028, 4029),
                ("limestone_rock1", 4029, 4030),
            ]
        );
        assert_eq!(rows[2]["empty_ids"][0]["alias"], "loc_4030");
        assert!(rows.iter().all(|row| {
            skill_of(row) == "mining"
                && row["publication"].is_null()
                && row["loc_ids"].as_array().unwrap().len() == 1
        }));
    }
}

#[test]
fn coverage_is_the_loaded_pin_and_unpublished_woods_still_hit() {
    let pin274 = methods(ClientRevision::R274, None);
    assert_eq!(
        pin274["coverage"],
        serde_json::json!([
            {
                "class": "unpublished",
                "table": "jungle_tree_table",
                "resource_key": "jungle",
                "reason": "unpublished wood"
            },
            {
                "class": "unpublished",
                "table": "burnt_tree_table",
                "resource_key": "burnt",
                "reason": "unpublished wood"
            },
            {
                "class": "conditional",
                "table": "achey_tree_table",
                "resource_key": "achey",
                "reason": "no supported consumer"
            },
            {
                "class": "conditional",
                "table": "hollow_tree_table",
                "resource_key": "hollow",
                "reason": "no supported consumer"
            },
            {
                "class": "revision-absent",
                "alias": "dungeon_tree_closed",
                "on_revision": 274,
                "other_pin_id": 5083,
                "copied": false,
                "reason": "289-only loc, not copied onto 274"
            },
            {
                "class": "revision-absent",
                "alias": "karam_dungeon_exit",
                "on_revision": 274,
                "other_pin_id": 5084,
                "copied": false,
                "reason": "289-only loc, not copied onto 274"
            }
        ])
    );
    let mining = methods(ClientRevision::R274, Some("mining"));
    assert_eq!(mining["coverage"], pin274["coverage"]);
    let loc_ids: Vec<i64> = rows(&pin274)
        .iter()
        .filter_map(|row| row.get("loc_ids"))
        .flat_map(|ids| ids.as_array().unwrap())
        .map(|id| id["id"].as_i64().unwrap())
        .collect();
    assert!(!loc_ids.contains(&5083));
    assert!(!loc_ids.contains(&5084));

    let pin289 = methods(ClientRevision::R289, Some("fishing"));
    assert_eq!(pin289["coverage"].as_array().unwrap().len(), 4);
    assert_eq!(
        pin289["coverage"],
        serde_json::Value::Array(pin274["coverage"].as_array().unwrap()[..4].to_vec())
    );
    assert!(pin289["coverage"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row.get("other_pin_id").is_none()));

    let jungle = resource(ClientRevision::R274, "jungle").unwrap();
    assert_eq!(rows(&jungle)[0]["publication"], "unpublished");
    assert_eq!(rows(&jungle)[0]["qualification"], "partial");
    assert_eq!(
        rows(&jungle)[0]["missing_transform"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    let burnt = resource(ClientRevision::R274, "burnt").unwrap();
    assert_eq!(rows(&burnt)[0]["publication"], "unpublished");
    assert_eq!(rows(&burnt)[0]["qualification"], "complete");

    let normal274 = resource(ClientRevision::R274, "normal").unwrap();
    assert_eq!(rows(&normal274)[0]["loc_ids"].as_array().unwrap().len(), 26);
    assert_eq!(rows(&normal274)[0]["qualification"], "complete");
    let normal289 = resource(ClientRevision::R289, "normal").unwrap();
    assert_eq!(rows(&normal289)[0]["loc_ids"].as_array().unwrap().len(), 28);
    assert_eq!(rows(&normal289)[0]["qualification"], "partial");
    let coal289 = resource(ClientRevision::R289, "coal").unwrap();
    assert!(rows(&coal289)[0]["loc_ids"]
        .as_array()
        .unwrap()
        .iter()
        .any(|id| id["alias"] == "misc_dummy_coalrock1" && id["id"] == 4676));
    let coal274 = resource(ClientRevision::R274, "coal").unwrap();
    assert!(rows(&coal274)[0]["loc_ids"]
        .as_array()
        .unwrap()
        .iter()
        .all(|id| id["alias"] != "misc_dummy_coalrock1"));
}

#[test]
fn fishing_rows_keep_ops_and_do_not_invent_locs() {
    let fish = methods(ClientRevision::R274, Some("fishing"));
    let rows = rows(&fish);
    assert!(rows.iter().all(|row| {
        row.get("loc_ids").is_none()
            && row.get("empty_ids").is_none()
            && row.get("resource_key").is_none()
            && row.get("table").is_none()
            && row.get("publication").is_none()
            && row.get("missing_transform").is_none()
            && row["level"].is_null()
            && row["output"].is_null()
            && skill_of(row) == "fishing"
    }));
    let fresh = rows
        .iter()
        .find(|row| row["category"] == "freshfish")
        .unwrap();
    assert_eq!(fresh["primary_op"], "Lure");
    assert_eq!(fresh["pair_op"], "Bait");
    let hidden = rows
        .iter()
        .find(|row| row["category"] == "unknown")
        .unwrap();
    assert_eq!(hidden["pair_op"], "hidden");
    let bare = rows
        .iter()
        .find(|row| row["category"] == "category_453")
        .unwrap();
    assert!(bare["pair_op"].is_null());
}

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
    std::sync::Arc::new(serde_json::from_str(raw).expect("selected data without gather_methods"))
}

#[test]
fn v2_methods_are_sync_helper_results_with_locked_errors() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const omitted = api.gatherMethods();
  const empty = api.gatherMethods({});
  const iron = api.gatherResource({ name: 'Iron' });
  globalThis.__probe = JSON.stringify({
    omittedOk: omitted.ok,
    omittedRows: omitted.value && omitted.value.rows.length,
    omittedCoverage: omitted.value && omitted.value.coverage.length,
    emptyRows: empty.value && empty.value.rows.length,
    ironThen: typeof iron.then,
    ironRows: iron.value && iron.value.rows.length,
    ironKey: iron.value && iron.value.rows[0].resource_key,
    nullArg: api.gatherMethods(null),
    stringArg: api.gatherMethods('woodcutting'),
    nullSkill: api.gatherMethods({ skill: null }),
    blankSkill: api.gatherMethods({ skill: '   ' }),
    woods: api.gatherMethods({ skill: 'woods' }),
    nullName: api.gatherResource({ name: null }),
    missingName: api.gatherResource({}),
    rocks: api.gatherResource({ name: 'Rocks' }),
    blankName: api.gatherResource({ name: '   ' }),
    namespace: api.gather,
    promise: omitted instanceof Promise,
  });
}
"#;
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let value = probe(src, revision);
        let coverage = if revision == ClientRevision::R274 {
            6
        } else {
            4
        };
        assert_eq!(value["omittedOk"], true, "{value:?}");
        assert_eq!(value["omittedRows"], 36, "{revision:?}");
        assert_eq!(value["omittedCoverage"], coverage, "{revision:?}");
        assert_eq!(value["emptyRows"], 36);
        assert_eq!(value["ironThen"], "undefined");
        assert_eq!(value["ironRows"], 1);
        assert_eq!(value["ironKey"], "iron");
        assert_eq!(value["nullArg"]["error"], "invalid-args");
        assert_eq!(value["stringArg"]["error"], "invalid-args");
        assert_eq!(value["nullSkill"]["error"], "invalid-args");
        assert_eq!(value["blankSkill"]["error"], "unknown-skill");
        assert_eq!(value["woods"]["error"], "unknown-skill");
        assert_eq!(value["nullName"]["error"], "invalid-args");
        assert_eq!(value["missingName"]["error"], "invalid-args");
        assert_eq!(value["rocks"]["error"], "unknown-resource");
        assert_eq!(value["blankName"]["error"], "unknown-resource");
        assert!(value["namespace"].is_null());
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
    assert_eq!(empty["omitted"]["error"], "missing-selected-data");
    assert_eq!(empty["empty"]["error"], "missing-selected-data");
    assert_eq!(empty["badSkill"]["error"], "missing-selected-data");
    assert_eq!(empty["missName"]["error"], "missing-selected-data");

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
    let bindings = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/load/bindings.rs"));
    assert!(
        !bindings.contains("register_function(\"__rs2b0t_gather_methods"),
        "gather query must not be a rustyscript JSON op"
    );
    let ops = bindings
        .split("const V2_OPS")
        .nth(1)
        .unwrap()
        .split("const OPTIONAL")
        .next()
        .unwrap();
    assert!(!ops.contains("gatherMethods"), "{ops}");
    assert!(!ops.contains("gatherResource"), "{ops}");
    let declared = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shim/declared_surface.js"
    ));
    assert!(declared.contains("export function bestAxe() { throw notImpl('bestAxe'); }"));
    assert!(declared.contains("export function bestPickaxe() { throw notImpl('bestPickaxe'); }"));
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
