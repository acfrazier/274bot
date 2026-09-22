//! Clue row over the landed schema-4 trail membership family. Not a tool table.
//! Not a clue machine and not a solve: the row read is the whole method.
use api::clue_facts::CluePin;
use client::io::ClientRevision;
use script::isolate_fb::{SceneEntityInput, TileInput};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};

fn facts(revision: ClientRevision) -> api::game_data::TrailFacts {
    api::game_data::for_revision(revision)
        .unwrap()
        .trails()
        .expect("landed family")
        .clone()
}

fn row(revision: ClientRevision, pin: CluePin<'_>) -> Result<serde_json::Value, &'static str> {
    let facts = facts(revision);
    api::clue_facts::clue_row(Some(&facts), pin)
}

#[test]
fn missing_family_is_the_method_token_before_the_key() {
    let blank = api::clue_facts::clue_row(None, CluePin::Alias("   "));
    assert_eq!(blank, Err("family-unavailable:trails"));
    let hit = api::clue_facts::clue_row(None, CluePin::Alias("trail_clue_hard_sextant028"));
    assert_eq!(hit, Err("family-unavailable:trails"));
    let id = api::clue_facts::clue_row(None, CluePin::Id(3554));
    assert_eq!(id, Err("family-unavailable:trails"));
    let zero = api::clue_facts::clue_row(None, CluePin::Id(0));
    assert_eq!(zero, Err("family-unavailable:trails"));
    assert_ne!(zero, Err("unknown-id"));
    assert_ne!(blank, Err("unknown-id"));
    assert_ne!(hit, Err("family-unavailable:quest_identity"));
    assert_ne!(hit, Err("family-unavailable:trail"));
    assert_ne!(
        api::clue_facts::clue_row(None, CluePin::Id(3554)),
        Ok(serde_json::json!({}))
    );
    assert_ne!(
        api::clue_facts::clue_row(None, CluePin::Id(3554)),
        Ok(serde_json::json!(null))
    );
}

#[test]
fn every_landed_row_is_copied_without_invented_fields() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let facts = facts(revision);
        assert_eq!(facts.rows.len(), 257);
        let mut with_access = 0;
        for landed in &facts.rows {
            let value = api::clue_facts::clue_row(Some(&facts), CluePin::Id(landed.id)).unwrap();
            let mut keys: Vec<&str> = value
                .as_object()
                .expect("row object")
                .keys()
                .map(String::as_str)
                .collect();
            keys.sort_unstable();
            let mut expected = vec!["alias", "id", "role", "params"];
            if landed.access.is_some() {
                with_access += 1;
                expected.push("access");
            }
            expected.sort_unstable();
            assert_eq!(keys, expected, "{}", landed.alias);
            assert_eq!(value["alias"].as_str(), Some(landed.alias.as_str()));
            assert_eq!(value["id"].as_i64(), Some(i64::from(landed.id)));
            assert_eq!(value["role"].as_str(), Some(landed.role.as_str()));
            assert_eq!(
                value.get("access").and_then(serde_json::Value::as_str),
                landed.access.as_deref()
            );
            let params = value["params"].as_array().expect("params always present");
            assert_eq!(params.len(), landed.params.len(), "{}", landed.alias);
            for (got, raw) in params.iter().zip(landed.params.iter()) {
                assert_eq!(got["key"].as_str(), Some(raw.key.as_str()));
                let text = got["value"]
                    .as_str()
                    .unwrap_or_else(|| panic!("coerced param value on {}: {got}", landed.alias));
                assert_eq!(text, raw.value);
            }
        }
        assert_eq!(with_access, 1);
    }
}

#[test]
fn the_bounded_row_is_access_constrained_and_not_playable() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let sextant = row(revision, CluePin::Id(3554)).unwrap();
        assert_eq!(sextant["alias"], "trail_clue_hard_sextant028");
        assert_eq!(sextant["role"], "clue");
        assert_eq!(sextant["access"], "constrained");
        assert!(sextant.get("rows").is_none(), "{sextant}");
        assert!(sextant.get("supported").is_none(), "{sextant}");
        assert!(sextant.get("playable").is_none(), "{sextant}");
        assert!(sextant.get("name").is_none(), "{sextant}");
        assert_eq!(
            sextant["params"],
            serde_json::json!([
                {
                    "key": "trail_desc",
                    "value": "02 degrees 46 minutes North|29 degrees 11 minutes East"
                },
                { "key": "trail_sextant", "value": "yes" },
                { "key": "trail_casket", "value": "trail_clue_hard_sextant028_casket" },
                { "key": "trail_coord", "value": "0_52_50_46_50" }
            ])
        );
        let by_alias = row(revision, CluePin::Alias("  Trail_Clue_Hard_Sextant028  ")).unwrap();
        assert_eq!(by_alias, sextant);
        let casket = row(revision, CluePin::Alias("trail_clue_hard_sextant028_casket")).unwrap();
        assert_eq!(casket["id"], 3555);
        assert_eq!(casket["role"], "casket");
        assert!(casket.get("access").is_none(), "{casket}");
        let plain = row(revision, CluePin::Id(2677)).unwrap();
        assert!(plain.get("access").is_none(), "{plain}");
    }
}

#[test]
fn params_stay_raw_strings_in_file_order_and_empty_arrays_stay() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let simple = row(revision, CluePin::Id(2677)).unwrap();
        assert_eq!(
            simple["params"][2],
            serde_json::json!({ "key": "trail_loc", "value": "^true" })
        );
        assert_eq!(
            simple["params"][1],
            serde_json::json!({ "key": "trail_coord", "value": "1_50_50_9_18" })
        );
        assert!(simple["params"][1]["value"].is_string(), "{simple}");
        for id in [3531, 2779] {
            let casket = row(revision, CluePin::Id(id)).unwrap();
            assert_eq!(casket["role"], "casket", "{id}");
            assert_eq!(casket["params"], serde_json::json!([]), "{id}");
            assert!(casket.get("params").is_some(), "{id} {casket}");
        }
        let anagram = row(revision, CluePin::Id(2841)).unwrap();
        assert_eq!(
            anagram["params"],
            serde_json::json!([{ "key": "trail_desc", "value": "Speak to Hazelmere." }])
        );
        assert!(anagram.get("npc").is_none(), "{anagram}");
        assert!(anagram.get("answer").is_none(), "{anagram}");
        assert!(anagram.get("grandtree_hazelmere").is_none(), "{anagram}");
    }
}

#[test]
fn non_members_and_challenge_rows_are_unknown_id() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        for id in [3533, 2842, 2844, 2846, 2850, 2852, 2854, 6859, 0, -1, i32::MAX] {
            assert_eq!(
                row(revision, CluePin::Id(id)),
                Err("unknown-id"),
                "{revision:?} id {id}"
            );
            assert_eq!(
                row(revision, CluePin::Alias(&id.to_string())),
                Err("unknown-id"),
                "{revision:?} alias {id}"
            );
        }
        for alias in [
            "trail_clue_hard_sextant017_casket",
            "trail_clue_medium_map002",
            "trail_clue_hard_sextant026",
            "trail_clue_medium_anagram001_challenge",
            "trail_clue_hard_sextant028_casket_challenge",
            "trail_clue_hard_sextant0289",
            "",
            "   ",
        ] {
            assert_eq!(
                row(revision, CluePin::Alias(alias)),
                Err("unknown-id"),
                "{revision:?} alias {alias:?}"
            );
        }
        assert_eq!(row(revision, CluePin::Alias("3554")), Err("unknown-id"));
        assert_eq!(row(revision, CluePin::Id(3554)).unwrap()["alias"], "trail_clue_hard_sextant028");
    }
}

#[test]
fn query_does_not_open_the_answers_or_the_writer() {
    let src = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../api/src/clue_facts.rs"
    ));
    assert!(!src.contains("challenge_answers"));
    assert!(!src.contains("challengeAnswer"));
    assert!(!src.contains("&SelectedGameData"));
    assert!(!src.contains("Serialize"));
    assert!(!src.contains("items()"));
    assert!(!src.contains("trail_coord"));
    assert!(!src.contains("\"supported\""));
    assert!(!src.contains("parse"));
}

/// The call-time pages the clue search machine reads on top of the pack page:
/// the posted `here` tile, the posted loc page, and the posted `hold || ours`
/// pair.
#[derive(Default)]
struct Scene<'a> {
    here: Option<script::isolate_fb::TileInput>,
    locs: &'a [script::isolate_fb::SceneEntityInput<'a>],
    hold: bool,
    ours: bool,
}

fn post_base(iso: &LoadIsolate, tick: u64) {
    post_page(iso, tick, &[]);
}

/// The same keyframe with a posted pack page: `(obj id, count)` rows in posted
/// order, the page the isolate packer posts as `snapshot.inv`.
fn post_page(iso: &LoadIsolate, tick: u64, page: &[(i32, i32)]) {
    post_scene(iso, tick, page, &Scene::default());
}

/// The same keyframe with the scene the search machine reads: the posted
/// `here` tile and the posted loc page, both marshalled into `next` at call
/// time, plus the posted `hold || ours` pair.
fn post_scene(iso: &LoadIsolate, tick: u64, page: &[(i32, i32)], scene: &Scene<'_>) {
    let rows: Vec<script::isolate_fb::ItemRowInput<'_>> = page
        .iter()
        .map(|(id, count)| script::isolate_fb::ItemRowInput {
            name: None,
            count: *count,
            id: *id,
            ops: &[],
            noted: false,
            cert: -1,
            component_id: -1,
            slot: -1,
        })
        .collect();
    let input = script::isolate_fb::SnapshotInput {
        tick,
        here: scene.here,
        ingame: true,
        inv: &rows,
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
        hold: scene.hold,
        ours: scene.ours,
        npcs: &[],
        locs: scene.locs,
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

/// The same probe over a posted pack page instead of an empty pack.
fn probe_page(src: &str, revision: ClientRevision, page: &[(i32, i32)]) -> serde_json::Value {
    let data = api::game_data::for_revision(revision).unwrap();
    let iso =
        LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data).unwrap();
    post_page(&iso, 1, page);
    iso.on_game_tick(1);
    let value = iso.probe("globalThis.__probe").unwrap();
    iso.join();
    serde_json::from_str(value.as_str().unwrap()).unwrap()
}

/// A schema-4 selected slot that decodes with no trail family at all.
fn data_without_trails() -> std::sync::Arc<api::game_data::SelectedGameData> {
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
    std::sync::Arc::new(serde_json::from_str(raw).expect("selected data without trails"))
}

#[test]
fn v2_clue_row_is_a_sync_helper_result() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const byId = api.clue.row({ id: 3554 });
  const byAlias = api.clue.row({ alias: '  Trail_Clue_Hard_Sextant028  ' });
  const extra = api.clue.row({ id: 3554, supported: true, access: 'open', name: 'x' });
  const casket = api.clue.row({ id: 3531 });
  globalThis.__probe = JSON.stringify({
    idOk: byId.ok,
    idThen: typeof byId.then,
    idAlias: byId.value && byId.value.alias,
    idRole: byId.value && byId.value.role,
    idAccess: byId.value && byId.value.access,
    idParams: byId.value && byId.value.params.length,
    idRows: byId.value ? Object.prototype.hasOwnProperty.call(byId.value, 'rows') : 'no-row',
    aliasSame: JSON.stringify(byAlias.value) === JSON.stringify(byId.value),
    extraAccess: extra.value && extra.value.access,
    extraSupported: extra.value
      ? Object.prototype.hasOwnProperty.call(extra.value, 'supported') : 'no-row',
    extraName: extra.value
      ? Object.prototype.hasOwnProperty.call(extra.value, 'name') : 'no-row',
    casketParams: casket.value && casket.value.params.length,
    rowType: typeof api.clue.row,
    flat: typeof api.clueRow,
    heldStepType: typeof api.clue.heldStep,
    begin: typeof api.clue.begin,
    next: typeof api.clue.next,
    packPlan: typeof api.clue.packPlan,
    hardKit: typeof api.clue.hardKit,
    challengeAnswer: typeof api.clue.challengeAnswer,
    deposit: typeof api.clue.deposit,
    retry: typeof api.clue.retry,
    noteDeath: typeof api.clue.noteDeath,
    quest: typeof api.quest,
    questionNamespace: api.clue && Object.keys(api.clue),
  });
}
"#;
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let value = probe(src, revision);
        assert_eq!(value["idOk"], true, "{revision:?} {value:?}");
        assert_eq!(value["idThen"], "undefined", "{value:?}");
        assert_eq!(value["idAlias"], "trail_clue_hard_sextant028", "{value:?}");
        assert_eq!(value["idRole"], "clue", "{value:?}");
        assert_eq!(value["idAccess"], "constrained", "{value:?}");
        assert_eq!(value["idParams"], 4, "{value:?}");
        assert_eq!(value["idRows"], false, "{value:?}");
        assert_eq!(value["aliasSame"], true, "{value:?}");
        assert_eq!(value["extraAccess"], "constrained", "{value:?}");
        assert_eq!(value["extraSupported"], false, "{value:?}");
        assert_eq!(value["extraName"], false, "{value:?}");
        assert_eq!(value["casketParams"], 0, "{value:?}");
        assert_eq!(value["rowType"], "function", "{value:?}");
        assert_eq!(value["flat"], "undefined", "{value:?}");
        assert_eq!(value["heldStepType"], "function", "{value:?}");
        for key in [
            "begin",
            "next",
        ] {
            assert_eq!(value[key], "function", "{key} {value:?}");
        }
        for key in ["challengeAnswer", "deposit", "retry", "noteDeath"] {
            assert_eq!(value[key], "undefined", "{key} {value:?}");
        }
        assert_eq!(value["packPlan"], "function", "{value:?}");
        assert_eq!(value["hardKit"], "function", "{value:?}");
        assert_eq!(value["quest"], "undefined", "{value:?}");
        assert_eq!(
            value["questionNamespace"],
            serde_json::json!(["row", "heldStep", "packPlan", "hardKit", "begin", "next"]),
            "{value:?}"
        );
    }
}

#[test]
fn v2_clue_row_rejects_a_converted_id() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__probe = JSON.stringify({
    stringId: api.clue.row({ id: '3554' }),
    fraction: api.clue.row({ id: 3554.5 }),
    above: api.clue.row({ id: 2147483648 }),
    below: api.clue.row({ id: -2147483649 }),
    huge: api.clue.row({ id: 1e21 }),
    bigint: api.clue.row({ id: BigInt(3554) }),
    boxed: api.clue.row({ id: new Number(3554) }),
    both: api.clue.row({ id: 3554, alias: 'trail_clue_hard_sextant028' }),
    neither: api.clue.row({}),
    omitted: api.clue.row(),
    array: api.clue.row([]),
    nullArg: api.clue.row(null),
    numberArg: api.clue.row(3554),
    aliasNumber: api.clue.row({ alias: 3554 }),
    aliasNull: api.clue.row({ alias: null }),
    stringAlias: api.clue.row({ alias: 'trail_clue_hard_sextant028' }),
    zero: api.clue.row({ id: 0 }),
    // -0 is a number and in range, but it is not an int32 to the V8 pin
    // re-check: invalid-args, not a 0 miss.
    negativeZero: api.clue.row({ id: -0 }),
    blankAlias: api.clue.row({ alias: '   ' }),
    emptyAlias: api.clue.row({ alias: '' }),
  });
}
"#;
    let value = probe(src, ClientRevision::R274);
    for key in [
        "stringId",
        "fraction",
        "above",
        "below",
        "huge",
        "bigint",
        "boxed",
        "both",
        "neither",
        "omitted",
        "array",
        "nullArg",
        "numberArg",
        "aliasNumber",
        "aliasNull",
        "negativeZero",
    ] {
        assert_eq!(value[key]["error"], "invalid-args", "{key} {value:?}");
    }
    for key in ["zero", "blankAlias", "emptyAlias"] {
        assert_eq!(value[key]["error"], "unknown-id", "{key} {value:?}");
    }
    assert_eq!(value["stringAlias"]["ok"], true, "{value:?}");
    assert_eq!(value["stringAlias"]["value"]["id"], 3554, "{value:?}");
    assert_eq!(value["stringAlias"]["value"]["access"], "constrained", "{value:?}");
}

#[test]
fn v2_clue_held_step_is_a_sync_helper_result_over_the_posted_page() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const step = api.clue.heldStep();
  // Zero parameters: an argument is ignored, not an inventory override.
  const extra = api.clue.heldStep([{ id: 2714, count: 1 }], 'held', 7);
  globalThis.__probe = JSON.stringify({
    ok: step.ok,
    then: typeof step.then,
    alias: step.value && step.value.alias,
    id: step.value && step.value.id,
    role: step.value && step.value.role,
    params: step.value && step.value.params,
    access: step.value && step.value.access,
    rows: step.value ? Object.prototype.hasOwnProperty.call(step.value, 'rows') : 'no-row',
    extraSame: JSON.stringify(extra) === JSON.stringify(step),
    type: typeof api.clue.heldStep,
  });
}
"#;
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        // A clue earlier in the pack does not beat a later casket, and a held
        // id that is not a membership row is skipped, not unknown-id.
        let value = probe_page(src, revision, &[(999_999, 1), (3554, 1), (3531, 1)]);
        assert_eq!(value["ok"], true, "{revision:?} {value:?}");
        assert_eq!(value["then"], "undefined", "{value:?}");
        assert_eq!(value["alias"], "trail_clue_hard_sextant016_casket", "{value:?}");
        assert_eq!(value["id"], 3531, "{value:?}");
        assert_eq!(value["role"], "casket", "{value:?}");
        assert_eq!(value["params"], serde_json::json!([]), "{value:?}");
        assert!(value["access"].is_null(), "casket omits access: {value:?}");
        assert_eq!(value["rows"], false, "{value:?}");
        assert_eq!(value["extraSame"], true, "{value:?}");
        assert_eq!(value["type"], "function", "{value:?}");
    }
}

#[test]
fn v2_clue_held_step_takes_the_first_held_step_and_else_none_held() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__probe = JSON.stringify({ step: api.clue.heldStep() });
}
"#;
    // Zero and negative counts are not held, so the clue behind the unheld
    // casket wins and carries the landed access field.
    let value = probe_page(src, ClientRevision::R274, &[(3531, 0), (0, 3), (3554, 1)]);
    assert_eq!(value["step"]["ok"], true, "{value:?}");
    assert_eq!(value["step"]["value"]["id"], 3554, "{value:?}");
    assert_eq!(
        value["step"]["value"]["alias"], "trail_clue_hard_sextant028",
        "{value:?}"
    );
    assert_eq!(value["step"]["value"]["role"], "clue", "{value:?}");
    assert_eq!(value["step"]["value"]["access"], "constrained", "{value:?}");
    assert_eq!(
        value["step"]["value"]["params"].as_array().map(Vec::len),
        Some(4),
        "{value:?}"
    );
    // Within one role the page decides, not the membership table: 3554 is
    // posted before 2713 and is the step even though 2713 is the earlier row.
    let value = probe_page(src, ClientRevision::R274, &[(3554, 1), (2713, 1)]);
    assert_eq!(value["step"]["value"]["id"], 3554, "{value:?}");
    // A held challenge id is not a membership row: an unrelated pack is
    // none-held, not unknown-id, and it is not an ok null.
    let value = probe_page(src, ClientRevision::R274, &[(2842, 1), (0, 3), (3531, -1)]);
    assert_eq!(value["step"]["ok"], false, "{value:?}");
    assert_eq!(value["step"]["error"], "none-held", "{value:?}");
    assert!(value["step"].get("value").is_none(), "{value:?}");
    // An empty page is the same none-held.
    let value = probe_page(src, ClientRevision::R274, &[]);
    assert_eq!(value["step"]["error"], "none-held", "{value:?}");
}

#[test]
fn v2_clue_held_step_prefers_the_slot_token_then_the_family_token() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__probe = JSON.stringify({ step: api.clue.heldStep() });
}
"#;
    // No selected slot: the slot token, even with a held casket.
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    post_page(&iso, 1, &[(3531, 1)]);
    iso.on_game_tick(1);
    let value: serde_json::Value =
        serde_json::from_str(iso.probe("globalThis.__probe").unwrap().as_str().unwrap()).unwrap();
    iso.join();
    assert_eq!(value["step"]["error"], "missing-selected-data", "{value:?}");

    // A selected slot without the trail family: the family token, before
    // none-held, even with an empty page.
    let iso = LoadIsolate::spawn_with_game_data(
        src.into(),
        LoadShape::NativeTick,
        vec![],
        data_without_trails(),
    )
    .unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let value: serde_json::Value =
        serde_json::from_str(iso.probe("globalThis.__probe").unwrap().as_str().unwrap()).unwrap();
    iso.join();
    assert_eq!(
        value["step"]["error"], "family-unavailable:trails",
        "{value:?}"
    );
}

#[test]
fn v2_invalid_args_before_missing_slot_and_missing_family() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__probe = JSON.stringify({
    omitted: api.clue.row(),
    empty: api.clue.row({}),
    both: api.clue.row({ id: 3554, alias: 'trail_clue_hard_sextant028' }),
    array: api.clue.row([]),
    stringId: api.clue.row({ id: '3554' }),
    fraction: api.clue.row({ id: 3554.5 }),
    hit: api.clue.row({ id: 3554 }),
    aliasHit: api.clue.row({ alias: 'trail_clue_hard_sextant028' }),
    blank: api.clue.row({ alias: '   ' }),
    zero: api.clue.row({ id: 0 }),
  });
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let empty: serde_json::Value =
        serde_json::from_str(iso.probe("globalThis.__probe").unwrap().as_str().unwrap()).unwrap();
    iso.join();
    for key in ["omitted", "empty", "both", "array", "stringId", "fraction"] {
        assert_eq!(empty[key]["error"], "invalid-args", "{key} {empty:?}");
    }
    for key in ["hit", "aliasHit", "blank", "zero"] {
        assert_eq!(
            empty[key]["error"], "missing-selected-data",
            "{key} {empty:?}"
        );
    }

    let iso = LoadIsolate::spawn_with_game_data(
        src.into(),
        LoadShape::NativeTick,
        vec![],
        data_without_trails(),
    )
    .unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let missing: serde_json::Value =
        serde_json::from_str(iso.probe("globalThis.__probe").unwrap().as_str().unwrap()).unwrap();
    iso.join();
    assert_eq!(missing["omitted"]["error"], "invalid-args", "{missing:?}");
    assert_eq!(missing["stringId"]["error"], "invalid-args", "{missing:?}");
    assert_eq!(missing["fraction"]["error"], "invalid-args", "{missing:?}");
    for key in ["hit", "aliasHit", "blank", "zero"] {
        assert_eq!(
            missing[key]["error"], "family-unavailable:trails",
            "{key} {missing:?}"
        );
    }
}

#[test]
fn example_clue_facts_v2_is_one_read_only_row_call() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("clue_facts_v2.ts");
    let src = std::fs::read_to_string(&path).expect("example source");
    assert!(!src.contains("request("));
    assert!(!src.contains("h.interact"));
    assert!(!src.contains("challengeAnswer"));
    assert!(!src.contains("deposit"));
    assert_eq!(src.matches("api.clue").count(), 1);
    let js = script::transpile_ts(&src).expect("transpile clue_facts_v2.ts");
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
        .find(|line| line.contains("\"alias\""))
        .unwrap_or_else(|| panic!("example logged a row; logs={logs:?}"));
    let row: serde_json::Value = serde_json::from_str(last).unwrap();
    assert_eq!(row["ok"], true, "{row:?}");
    assert_eq!(row["value"]["id"], 3554);
    assert_eq!(row["value"]["alias"], "trail_clue_hard_sextant028");
    assert_eq!(row["value"]["access"], "constrained");
    assert!(row["value"].get("supported").is_none(), "{row:?}");
}

#[test]
fn example_clue_held_step_v2_is_one_read_only_held_step_call() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("clue_held_step_v2.ts");
    let src = std::fs::read_to_string(&path).expect("example source");
    assert!(!src.contains("request("));
    assert!(!src.contains("h.interact"));
    assert!(!src.contains("challengeAnswer"));
    assert!(!src.contains("deposit"));
    assert_eq!(src.matches("api.clue").count(), 1);
    assert_eq!(src.matches("heldStep").count(), 1);
    let js = script::transpile_ts(&src).expect("transpile clue_held_step_v2.ts");
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
        .find(|line| line.contains("\"none-held\""))
        .unwrap_or_else(|| panic!("example logged the held step; logs={logs:?}"));
    let step: serde_json::Value = serde_json::from_str(last).unwrap();
    // The example injects no page: the keyframe pack holds nothing.
    assert_eq!(step["ok"], false, "{step:?}");
    assert_eq!(step["error"], "none-held", "{step:?}");
    assert!(step.get("value").is_none(), "{step:?}");
}

#[test]
fn v2_clue_begin_and_next_drive_the_machine_over_the_posted_page() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  // Extra begin keys are ignored and nothing but the token is captured.
  const begin = api.clue.begin({ name: 'ignored', enabled: false, resume: false });
  const token = begin.ok ? begin.value.token : null;
  const gate = begin.ok ? api.clue.next({ token: token }) : null;
  const denied = begin.ok ? api.clue.next({ token: token, resume: false }) : null;
  // `reply` is not the answer slot: the gate is still open and re-asks.
  const replied = begin.ok ? api.clue.next({ token: token, reply: true }) : null;
  const enabled = begin.ok ? api.clue.next({ token: token, resume: true }) : null;
  const status = begin.ok ? api.clue.next({ token: token }) : null;
  const idle = begin.ok ? api.clue.next({ token: token }) : null;
  const dead = begin.ok ? api.clue.next({ token: token + 7 }) : null;
  globalThis.__probe = JSON.stringify({
    beginThen: typeof begin.then,
    beginKeys: begin.value ? Object.keys(begin.value) : 'no-value',
    tokenType: typeof token,
    gate, denied, replied, enabled, status, idle, dead,
    badResume: api.clue.next({ token: 1, resume: 'yes' }),
    badToken: api.clue.next({}),
    positional: api.clue.next(1, true),
    arrayArg: api.clue.next([]),
  });
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data)
        .unwrap();
    post_page(&iso, 1, &[(3554, 1)]);
    iso.on_game_tick(1);
    let value: serde_json::Value =
        serde_json::from_str(iso.probe("globalThis.__probe").unwrap().as_str().unwrap()).unwrap();
    let interacts = iso.drain_interacts();
    iso.join();

    assert_eq!(value["beginThen"], "undefined", "{value:?}");
    assert_eq!(value["beginKeys"], serde_json::json!(["token"]), "{value:?}");
    assert_eq!(value["tokenType"], "number", "{value:?}");
    let token = &value["gate"]["token"];
    assert!(token.is_number(), "{value:?}");
    for (key, kind) in [
        ("gate", "callback.enabled"),
        ("denied", "wait"),
        ("replied", "callback.enabled"),
        ("enabled", "callback.log"),
        ("status", "callback.setStatus"),
        ("idle", "wait"),
    ] {
        assert_eq!(value[key]["ok"], true, "{key} {value:?}");
        assert_eq!(value[key]["status"], "continue", "{key} {value:?}");
        assert_eq!(value[key]["kind"], kind, "{key} {value:?}");
        assert_eq!(&value[key]["token"], token, "{key} {value:?}");
        assert!(value[key].get("error").is_none(), "{key} {value:?}");
    }
    // A false answer idles the session without a message, and the progress
    // lines carry the landed identity only.
    assert!(value["denied"].get("message").is_none(), "{value:?}");
    let message = value["enabled"]["message"].as_str().unwrap_or("");
    assert!(message.contains("trail_clue_hard_sextant028"), "{value:?}");
    assert!(message.contains("3554"), "{value:?}");
    assert!(!message.contains("clue solved"), "{value:?}");
    assert!(
        !value["status"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("clue solved"),
        "{value:?}"
    );
    // A token that is not the session's is the error object, never undefined.
    assert_eq!(value["dead"]["ok"], false, "{value:?}");
    assert_eq!(value["dead"]["error"], "stale", "{value:?}");
    assert_eq!(value["dead"]["status"], serde_json::Value::Null, "{value:?}");
    for key in ["badResume", "badToken", "positional", "arrayArg"] {
        assert_eq!(value[key]["error"], "invalid-args", "{key} {value:?}");
    }
    assert!(
        interacts.is_empty(),
        "the clue machine pushes no interact: {interacts:?}"
    );
}

#[test]
fn v2_clue_next_keeps_none_held_when_the_live_session_loses_its_step() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__tick = (globalThis.__tick || 0) + 1;
  if (globalThis.__tick === 1) {
    const begin = api.clue.begin();
    globalThis.__begin = begin;
    globalThis.__token = begin.ok ? begin.value.token : null;
    return;
  }
  if (globalThis.__tick === 2) {
    // The page still holds something, just nothing that is a membership row.
    globalThis.__dropped = api.clue.next({ token: globalThis.__token });
    return;
  }
  // The membership row is back, but the step's token is already dead.
  globalThis.__after = api.clue.next({ token: globalThis.__token });
  globalThis.__probe = JSON.stringify({
    token: globalThis.__token,
    begin: globalThis.__begin,
    dropped: globalThis.__dropped,
    after: globalThis.__after,
  });
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data)
        .unwrap();
    post_page(&iso, 1, &[(3554, 1)]);
    iso.on_game_tick(1);
    // A held challenge id is not a membership row: the live step is gone.
    post_page(&iso, 2, &[(2842, 1)]);
    iso.on_game_tick(2);
    post_page(&iso, 3, &[(3554, 1)]);
    iso.on_game_tick(3);
    let value: serde_json::Value =
        serde_json::from_str(iso.probe("globalThis.__probe").unwrap().as_str().unwrap()).unwrap();
    let interacts = iso.drain_interacts();
    iso.join();

    assert!(value["token"].is_number(), "{value:?}");
    assert_eq!(value["begin"]["ok"], true, "{value:?}");
    assert_eq!(value["begin"]["value"]["token"], value["token"], "{value:?}");
    // The identify family's own reason survives the wrapper on `next`: not
    // `stale`, and not a continue step.
    assert_eq!(value["dropped"]["ok"], false, "{value:?}");
    assert_eq!(value["dropped"]["error"], "none-held", "{value:?}");
    assert!(value["dropped"].get("value").is_none(), "{value:?}");
    // `none-held` aborts: the old token is dead even with the row held again.
    assert_eq!(value["after"]["ok"], false, "{value:?}");
    assert_eq!(value["after"]["error"], "stale", "{value:?}");
    assert!(
        interacts.is_empty(),
        "the clue machine pushes no interact: {interacts:?}"
    );
}

/// One posted loc row: the fields the picker reads, and the same defaults the
/// materializer writes beside them.
fn scene_loc<'a>(
    id: i32,
    x: i32,
    z: i32,
    level: i32,
    actions: &'a [String],
) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index: 7,
        id,
        name: None,
        x,
        z,
        level,
        distance: 1,
        health: 0,
        max_health: 0,
        in_combat: false,
        animating: false,
        actions,
        reachable: true,
        reachable_adj: true,
        combat_level: 0,
        target_kind: 0,
        target_index: -1,
        size: 1,
        nx: x,
        nz: z,
    }
}

/// The public `api.clue.next` path over a posted page plus a posted scene: the
/// machine's own `walk` / `loc` kinds reach the interact drain as
/// `InteractReq::Walk` / `InteractReq::Loc`, which no module test can see.
#[test]
fn v2_clue_search_row_walks_then_locates_over_the_posted_scene() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__runs = (globalThis.__runs || 0) + 1;
  if (globalThis.__runs === 1) {
    const begin = api.clue.begin();
    globalThis.__token = begin.ok ? begin.value.token : null;
    globalThis.__steps = [
      begin,
      api.clue.next({ token: globalThis.__token }),
      api.clue.next({ token: globalThis.__token, resume: true }),
      api.clue.next({ token: globalThis.__token }),
      // No posted `here`: the search row waits instead of walking blind.
      api.clue.next({ token: globalThis.__token }),
    ];
    return;
  }
  globalThis.__steps.push(api.clue.next({ token: globalThis.__token }));
  if (globalThis.__steps.length === 10) {
    let requested = null;
    try { api.request({ op: 'loc' }); requested = 'ok'; }
    catch (e) { requested = String(e && (e.message || e)); }
    globalThis.__probe = JSON.stringify({
      token: globalThis.__token,
      runs: globalThis.__runs,
      steps: globalThis.__steps,
      locRequest: requested,
      snapshotLocs: typeof api.snapshot.locs,
    });
  }
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data)
        .unwrap();
    let page = [(2677, 1)];

    // Tick 1: the identified `trail_clue_easy_simple001` search row with no
    // posted scene at all. Nothing to measure means no verb.
    post_page(&iso, 1, &page);
    iso.on_game_tick(1);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "a missing `here` is a wait, not a walk"
    );

    // Tick 2: `here` far from the decoded 1_50_50_9_18 tile.
    post_scene(
        &iso,
        2,
        &page,
        &Scene {
            here: Some(TileInput {
                x: 3200,
                z: 3218,
                level: 1,
            }),
            ..Scene::default()
        },
    );
    iso.on_game_tick(2);
    // `on_game_tick` is fire-and-forget: the probe is the barrier that proves
    // the tick ran and its interact batch was forwarded.
    assert!(iso.probe("true").is_ok());
    let walk = iso.drain_interacts();
    assert_eq!(
        walk,
        vec![InteractReq::Walk {
            x: 3209,
            z: 3218,
            level: 1,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 0,
        }],
        "the walk is the decoded tile with default flags: {walk:?}"
    );

    // Tick 3: arrived, with junk rows, a wrong level and a far row ahead of
    // the row the picker takes. The verb keeps the posted tile and id.
    let wrong_level = vec!["Search".to_string()];
    let too_far = vec!["Search".to_string()];
    let found = vec!["sEaRcH".to_string()];
    post_scene(
        &iso,
        3,
        &page,
        &Scene {
            here: Some(TileInput {
                x: 3209,
                z: 3218,
                level: 1,
            }),
            locs: &[
                scene_loc(20, 3209, 3218, 2, &wrong_level),
                scene_loc(21, 3211, 3218, 1, &too_far),
                scene_loc(25, 3210, 3217, 1, &found),
            ],
            ..Scene::default()
        },
    );
    iso.on_game_tick(3);
    assert!(iso.probe("true").is_ok());
    let picked = iso.drain_interacts();
    assert_eq!(
        picked,
        vec![InteractReq::Loc {
            x: 3210,
            z: 3217,
            level: 1,
            action: "Search".to_string(),
            id: Some(25),
        }],
        "the pick is the posted row, id included: {picked:?}"
    );

    // Tick 4: arrived with an empty loc page: a wait, and the token stays
    // live for the walk tick 7 still gets.
    post_scene(
        &iso,
        4,
        &page,
        &Scene {
            here: Some(TileInput {
                x: 3209,
                z: 3218,
                level: 1,
            }),
            ..Scene::default()
        },
    );
    iso.on_game_tick(4);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "no searchable loc is a wait, never abandon"
    );

    // Tick 5: the posted `ours` interrupt with an unfrozen clock: yield, and
    // no verb rides along with it.
    let locs = [scene_loc(25, 3210, 3217, 1, &found)];
    post_scene(
        &iso,
        5,
        &page,
        &Scene {
            here: Some(TileInput {
                x: 3200,
                z: 3218,
                level: 1,
            }),
            locs: &locs,
            ours: true,
            ..Scene::default()
        },
    );
    iso.on_game_tick(5);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "yield emits no walk and no loc"
    );

    // Tick 6: the posted `hold` freezes this machine's clock and is a
    // paint-only tick — the script never runs — so the fully walkable scene
    // still reaches the drain with nothing.
    post_scene(
        &iso,
        6,
        &page,
        &Scene {
            here: Some(TileInput {
                x: 3200,
                z: 3218,
                level: 1,
            }),
            locs: &locs,
            hold: true,
            ..Scene::default()
        },
    );
    iso.on_game_tick(6);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "a frozen call emits no walk and no loc"
    );
    assert_eq!(
        iso.probe("String(globalThis.__runs)").unwrap().as_str(),
        Some("5"),
        "the posted hold skipped the tick's script, and it skipped no verb"
    );

    // Tick 7: thawed, the same far `here` walks again.
    post_scene(
        &iso,
        7,
        &page,
        &Scene {
            here: Some(TileInput {
                x: 3200,
                z: 3218,
                level: 1,
            }),
            locs: &locs,
            ..Scene::default()
        },
    );
    iso.on_game_tick(7);
    assert!(iso.probe("true").is_ok());
    let walked_again = iso.drain_interacts();
    assert_eq!(
        walked_again,
        vec![InteractReq::Walk {
            x: 3209,
            z: 3218,
            level: 1,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 0,
        }],
        "{walked_again:?}"
    );

    let probed = iso.probe("globalThis.__probe").unwrap();
    let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
    iso.join();
    assert!(value["token"].is_number(), "{value:?}");
    // Seven dispatches, six script runs: the `hold` tick is paint-only.
    assert_eq!(value["runs"], 6, "{value:?}");
    let steps = value["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 10, "{value:?}");
    let token = &value["token"];
    // The landed envelope is unchanged ahead of the search verbs.
    for (index, kind) in [
        (1, "callback.enabled"),
        (2, "callback.log"),
        (3, "callback.setStatus"),
        (4, "wait"),
        (5, "walk"),
        (6, "loc"),
        (7, "wait"),
        (8, "yield"),
        (9, "walk"),
    ] {
        assert_eq!(steps[index]["kind"], kind, "{index} {value:?}");
    }
    for index in 1..steps.len() {
        let step = &steps[index];
        assert_eq!(step["ok"], true, "{index} {step}");
        assert_eq!(step["status"], "continue", "{index} {step}");
        assert_eq!(step["token"], *token, "{index} {step}");
        assert!(step.get("error").is_none(), "{index} {step}");
    }
    // The decode pin on the public path: trail_clue_easy_simple001's selected
    // 1_50_50_9_18 token is (3209, 3218, 1).
    for index in [5, 9] {
        assert_eq!(steps[index]["x"], 3209, "{index} {value:?}");
        assert_eq!(steps[index]["z"], 3218, "{index} {value:?}");
        assert_eq!(steps[index]["level"], 1, "{index} {value:?}");
    }
    assert_eq!(steps[6]["action"], "Search", "{value:?}");
    assert_eq!(steps[6]["id"], 25, "{value:?}");
    assert_eq!(steps[6]["x"], 3210, "{value:?}");
    assert_eq!(steps[6]["z"], 3217, "{value:?}");
    assert_eq!(steps[6]["level"], 1, "{value:?}");
    // The message lines stay the landed identity, never an answer.
    let logged = steps[2]["message"].as_str().unwrap_or("");
    assert!(logged.contains("trail_clue_easy_simple001"), "{value:?}");
    assert!(logged.contains("2677"), "{value:?}");
    // No completion, no abandonment, and no coordinate on the row read.
    let text = value.to_string();
    for forbidden in ["clue solved", "abandon", "supplies-needed", "ownsEquipment"] {
        assert!(!text.contains(forbidden), "{value:?}");
    }
    // Loc stays unpublished: not a `request()` op, and not on `api.snapshot`.
    assert!(
        value["locRequest"]
            .as_str()
            .unwrap_or("")
            .contains("not impl"),
        "{value:?}"
    );
    assert_eq!(value["snapshotLocs"], "undefined", "{value:?}");
}

/// One held casket Open as the drain sees it: the selected item display name
/// the host resolves, and the frozen action. No row id, no tile.
fn casket_open() -> InteractReq {
    InteractReq::Held {
        name: "Casket".to_string(),
        action: "Open".to_string(),
    }
}

/// The public `api.clue.next` path over a posted casket page: the machine's own
/// `held` kind reaches the interact drain as `InteractReq::Held` — which no
/// module test can see — and it repeats while the same casket id stays held.
/// The page is the sextant028 casket held beside its own constrained 3554
/// clue, so the Open is never 3554 play.
#[test]
fn v2_clue_casket_row_opens_the_held_item_over_the_posted_page() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__runs = (globalThis.__runs || 0) + 1;
  if (globalThis.__runs === 1) {
    const begin = api.clue.begin();
    globalThis.__token = begin.ok ? begin.value.token : null;
    globalThis.__steps = [
      begin,
      api.clue.next({ token: globalThis.__token }),
      api.clue.next({ token: globalThis.__token, resume: true }),
      api.clue.next({ token: globalThis.__token }),
      // The landed report comes first: these two are the casket's own Open,
      // and they repeat while the same casket id stays in the pack.
      api.clue.next({ token: globalThis.__token }),
      api.clue.next({ token: globalThis.__token }),
    ];
    return;
  }
  globalThis.__steps.push(api.clue.next({ token: globalThis.__token }));
  if (globalThis.__runs === 5) {
    globalThis.__probe = JSON.stringify({
      token: globalThis.__token,
      runs: globalThis.__runs,
      steps: globalThis.__steps,
    });
  }
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data)
        .unwrap();
    // The casket and the constrained clue it belongs to both held: identify is
    // casket-first, so every Open below is the casket's and 3554 is never one.
    let pair = [(3554, 1), (3555, 1)];

    // Tick 1: the landed report, then Open — twice, because the same casket id
    // is still held on this call's page.
    post_page(&iso, 1, &pair);
    iso.on_game_tick(1);
    assert!(iso.probe("true").is_ok());
    let opened = iso.drain_interacts();
    assert_eq!(
        opened,
        vec![casket_open(), casket_open()],
        "each held call opens the casket: {opened:?}"
    );

    // Tick 2: the posted `ours` interrupt, unfrozen: yield, and no Open rides
    // along with it.
    post_scene(
        &iso,
        2,
        &pair,
        &Scene {
            ours: true,
            ..Scene::default()
        },
    );
    iso.on_game_tick(2);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "yield emits no held Open"
    );

    // Tick 3: the posted `hold` freezes this machine's clock and is a
    // paint-only tick — the script never runs — so no Open is re-sent.
    post_scene(
        &iso,
        3,
        &pair,
        &Scene {
            hold: true,
            ..Scene::default()
        },
    );
    iso.on_game_tick(3);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "a frozen call emits no held Open"
    );
    assert_eq!(
        iso.probe("String(globalThis.__runs)").unwrap().as_str(),
        Some("2"),
        "the posted hold skipped the tick's script, and it skipped no verb"
    );

    // Tick 4: thawed, the casket still held: the Open is back.
    post_page(&iso, 4, &pair);
    iso.on_game_tick(4);
    assert!(iso.probe("true").is_ok());
    assert_eq!(iso.drain_interacts(), vec![casket_open()], "the Open repeats");

    // Tick 5: the casket gone, the constrained clue left alone. The gate
    // re-arms for 3554 — a different held row — and the packed clue is not a
    // held Open.
    post_page(&iso, 5, &[(3554, 1)]);
    iso.on_game_tick(5);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "3554 is a clue, not a casket: nothing is opened for it"
    );

    // Tick 6: the casket and every other membership row are gone. The live
    // session reports its own identify token and dies — that is not trail
    // completion, and it is not a collect.
    post_page(&iso, 6, &[]);
    iso.on_game_tick(6);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "a lost membership pushes nothing"
    );

    let probed = iso.probe("globalThis.__probe").unwrap();
    let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
    iso.join();

    assert!(value["token"].is_number(), "{value:?}");
    // Five script runs: the `hold` tick is paint-only, and it ran no step.
    assert_eq!(value["runs"], 5, "{value:?}");
    let steps = value["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 10, "{value:?}");
    let token = &value["token"];
    assert_eq!(steps[0]["ok"], true, "{value:?}");
    assert_eq!(
        steps[0]["value"]["token"], *token,
        "the begin keeps its own token: {value:?}"
    );
    for (index, kind) in [
        (1, "callback.enabled"),
        (2, "callback.log"),
        (3, "callback.setStatus"),
        (4, "held"),
        (5, "held"),
        (6, "yield"),
        (7, "held"),
        (8, "callback.enabled"),
    ] {
        assert_eq!(steps[index]["kind"], kind, "{index} {value:?}");
    }
    for index in 1..steps.len() - 1 {
        let step = &steps[index];
        assert_eq!(step["ok"], true, "{index} {step}");
        assert_eq!(step["status"], "continue", "{index} {step}");
        assert_eq!(step["token"], *token, "{index} {step}");
        assert!(step.get("error").is_none(), "{index} {step}");
    }
    // The empty page after the casket is the identify family's own refusal,
    // never a continue kind and never a `done`.
    let lost = &steps[9];
    assert_eq!(lost["ok"], false, "{value:?}");
    assert_eq!(lost["error"], "none-held", "{value:?}");
    assert!(lost.get("value").is_none(), "{value:?}");
    assert_eq!(lost["status"], serde_json::Value::Null, "{value:?}");
    for index in [4, 5, 7] {
        let step = &steps[index];
        assert_eq!(step["name"], "Casket", "{index} {value:?}");
        assert_eq!(step["action"], "Open", "{index} {value:?}");
        // The name is the identity: no bound row id and no tile rides along.
        for absent in ["id", "x", "z", "level"] {
            assert!(step.get(absent).is_none(), "{index} {absent} {step}");
        }
    }
    // The report ahead of the Open is the casket row's own, never the clue's.
    let logged = steps[2]["message"].as_str().unwrap_or("");
    assert!(
        logged.contains("trail_clue_hard_sextant028_casket"),
        "{value:?}"
    );
    assert!(logged.contains("3555"), "{value:?}");
    assert!(!logged.contains("3554"), "{value:?}");
    assert!(!logged.contains("clue solved"), "{value:?}");
    let text = value.to_string();
    for forbidden in ["clue solved", "abandon", "supplies-needed", "ownsEquipment"] {
        assert!(!text.contains(forbidden), "{value:?}");
    }
}

/// The rows that are not search members stay identified then idle even with a
/// fully walkable posted scene: packed 3554 is `access: "constrained"`, 2831
/// is a desc-only frozen `keyFrom` riddle, 2713 is a coord-only map, and 2722
/// is a clue row with no params at all — and none of them is a held casket,
/// so none of them opens anything.
#[test]
fn v2_clue_idle_rows_never_walk_or_search() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__tick = (globalThis.__tick || 0) + 1;
  if (globalThis.__tick === 1) {
    const begin = api.clue.begin();
    globalThis.__token = begin.ok ? begin.value.token : null;
    globalThis.__steps = [
      begin,
      api.clue.next({ token: globalThis.__token }),
      api.clue.next({ token: globalThis.__token, resume: true }),
      api.clue.next({ token: globalThis.__token }),
    ];
    return;
  }
  globalThis.__steps.push(api.clue.next({ token: globalThis.__token }));
  if (globalThis.__tick === 4) {
    globalThis.__probe = JSON.stringify({ token: globalThis.__token, steps: globalThis.__steps });
  }
}
"#;
    for id in [3554, 2722, 2831, 2713] {
        let actions = vec!["Search".to_string()];
        let locs = [scene_loc(25, 3209, 3218, 1, &actions)];
        let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
        let iso =
            LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data)
                .unwrap();
        let page = [(id, 1)];
        for tick in 1..=4 {
            // Arrived on the first tick and far on the rest: neither is a
            // verb for a row that is not a search membership.
            let here = if tick == 1 {
                TileInput {
                    x: 3209,
                    z: 3218,
                    level: 1,
                }
            } else {
                TileInput {
                    x: 3100,
                    z: 3300,
                    level: 1,
                }
            };
            post_scene(
                &iso,
                tick,
                &page,
                &Scene {
                    here: Some(here),
                    locs: &locs,
                    ..Scene::default()
                },
            );
            iso.on_game_tick(tick);
            // The tick is fire-and-forget; the probe is this tick's barrier.
            assert!(iso.probe("true").is_ok());
        }
        let probed = iso.probe("globalThis.__probe").unwrap();
        let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
        let interacts = iso.drain_interacts();
        iso.join();
        assert!(interacts.is_empty(), "{id} pushed interact: {interacts:?}");
        let steps = value["steps"].as_array().expect("steps");
        assert_eq!(steps.len(), 7, "{id} {value:?}");
        assert_eq!(steps[0]["ok"], true, "{id} {value:?}");
        for step in &steps[4..] {
            assert_eq!(step["kind"], "wait", "{id} {step}");
            assert_eq!(step["token"], value["token"], "{id} {step}");
            assert!(step.get("x").is_none(), "{id} {step}");
            assert!(step.get("action").is_none(), "{id} {step}");
        }
    }
}

#[test]
fn v2_clue_begin_keeps_family_absence_apart_from_none_held() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__probe = JSON.stringify({
    emptyPage: api.clue.begin(),
    heldPair: api.clue.begin({ held: [[3554, 1]] }),
  });
}
"#;
    let iso = LoadIsolate::spawn_with_game_data(
        src.into(),
        LoadShape::NativeTick,
        vec![],
        data_without_trails(),
    )
    .unwrap();
    post_page(&iso, 1, &[(3554, 1)]);
    iso.on_game_tick(1);
    let value: serde_json::Value =
        serde_json::from_str(iso.probe("globalThis.__probe").unwrap().as_str().unwrap()).unwrap();
    let interacts = iso.drain_interacts();
    iso.join();
    // Family absence is the method token: not `none-held`, and no live token.
    for key in ["emptyPage", "heldPair"] {
        assert_eq!(value[key]["ok"], false, "{key} {value:?}");
        assert_eq!(value[key]["error"], "family-unavailable:trails", "{key} {value:?}");
        assert!(value[key].get("value").is_none(), "{key} {value:?}");
    }
    assert!(
        interacts.is_empty(),
        "the clue machine pushes no interact: {interacts:?}"
    );
}
