//! Clue row over the landed schema-4 trail membership family. Not a tool table.
//! Not a clue machine and not a solve: the row read is the whole method.
use api::clue_facts::CluePin;
use client::io::ClientRevision;
use script::isolate_fb::{SceneEntityInput, StatInput, TileInput, VarpInput};
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

/// One posted puzzle board on a keyframe: the SNAP-shaped page the plan reads
/// — the identified component, its observed slot count and its sparse rows —
/// plus the board session generation the click rides.
#[derive(Clone, Copy)]
struct PostedPuzzle<'a> {
    component_id: i32,
    size: i32,
    items: &'a [script::isolate_fb::ItemRowInput<'a>],
    generation: u64,
}

/// The call-time pages the clue search machine reads on top of the pack page:
/// the posted `here` tile, the posted loc page, and the posted `hold || ours`
/// pair. The collect arm adds the posted ground page, the posted main modal id
/// and the display names on the pack page the Drop resolves. The guarded
/// encounter adds the posted npc page, the local-player slot and target pair,
/// the posted overlay varps and the posted stat rows.
struct Scene<'a> {
    here: Option<script::isolate_fb::TileInput>,
    locs: &'a [script::isolate_fb::SceneEntityInput<'a>],
    hold: bool,
    ours: bool,
    /// The posted ground page: the same `SceneEntity` shape as `locs`.
    ground: &'a [script::isolate_fb::SceneEntityInput<'a>],
    /// The posted main modal id on this keyframe.
    main_modal_id: i32,
    /// Display names for the page rows, by obj id. The identify page stays the
    /// `(id, count)` pair; this is the `snapshot.inv` name the Drop resolves.
    names: &'a [(i32, &'a str)],
    /// The posted npc page the guarded encounter observes after its Dig.
    npcs: &'a [script::isolate_fb::SceneEntityInput<'a>],
    /// The posted varps: the encounter reads the selected Protect from Magic
    /// overlay — index 95 — out of this page.
    varps: &'a [script::isolate_fb::VarpInput],
    /// The posted stat rows: the encounter reads the `hitpoints` effective out
    /// of this page for its mid-fight wait.
    stats: &'a [script::isolate_fb::StatInput<'a>],
    /// The posted local-player table slot the `targetsMe` read compares with.
    self_slot: i32,
    /// The posted local-player target pair. A page with no target posts
    /// `(0, -1)` the way the native page does.
    self_target_kind: i32,
    self_target_index: i32,
    /// The posted puzzle board: the page the held box's plan reads, and the
    /// generation its click rides. `None` posts no board table at all, so the
    /// isolate keeps its last one.
    puzzle: Option<PostedPuzzle<'a>>,
    /// The posted chat modal the talk arm reads: `-1` is the closed one the
    /// page posts itself, and any other id is an open chat no Talk-to may ride.
    chat_modal_id: i32,
    /// The posted `chat_continue`: the other half of the landed `dialog_ready`.
    chat_continue: bool,
    /// The posted count dialog: the talk arm answers only behind a posted
    /// `true`, so this is what the challenge step's answer rides.
    count_dialog_open: bool,
}

impl Default for Scene<'_> {
    fn default() -> Self {
        Self {
            here: None,
            locs: &[],
            hold: false,
            ours: false,
            ground: &[],
            main_modal_id: 0,
            names: &[],
            npcs: &[],
            varps: &[],
            stats: &[],
            self_slot: 0,
            self_target_kind: 0,
            self_target_index: -1,
            puzzle: None,
            chat_modal_id: -1,
            chat_continue: false,
            count_dialog_open: false,
        }
    }
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
            name: scene
                .names
                .iter()
                .find(|(row_id, _)| row_id == id)
                .map(|(_, name)| *name),
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
        stats: scene.stats,
        booths: &[],
        banks: &[],
        bank: &[],
        bank_side: &[],
        bank_open: false,
        bank_loaded: false,
        bank_generation: 0,
        count_dialog_open: scene.count_dialog_open,
        withdraw_x_result_seq: 0,
        withdraw_x_result: false,
        withdraw_load_result_seq: 0,
        withdraw_load_result: false,
        bank_op_result_seq: 0,
        bank_op_result: false,
        hold: scene.hold,
        ours: scene.ours,
        npcs: scene.npcs,
        locs: scene.locs,
        players: &[],
        ground: scene.ground,
        equipment: &[],
        chat_open: false,
        chat_continue: scene.chat_continue,
        chat_text: None,
        chat_options: &[],
        side_tab: -1,
        varps: scene.varps,
        combat_styles: &[],
        run_energy: 0,
        run_enabled: false,
        retaliate_enabled: false,
        my_name: None,
        in_combat: false,
        animating: false,
        main_modal_id: scene.main_modal_id,
        chat_modal_id: scene.chat_modal_id,
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
        self_slot: scene.self_slot,
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
        self_target_kind: scene.self_target_kind,
        self_target_index: scene.self_target_index,
        widgets: &[],
    };
    let native = script::isolate_fb::NativeFactsInput {
        puzzle_board: scene
            .puzzle
            .as_ref()
            .map(|puzzle| script::isolate_fb::PuzzleBoardInput {
                component_id: puzzle.component_id,
                size: puzzle.size,
                items: puzzle.items,
                generation: puzzle.generation,
            }),
        ..script::isolate_fb::NativeFactsInput::default()
    };
    iso.post_snapshot(script::isolate_fb::encode_snapshot_with_native(&input, native));
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
    post_page(&iso, 1, &[(2722, 1)]);
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
    assert!(message.contains("trail_clue_hard_map001"), "{value:?}");
    assert!(message.contains("2722"), "{value:?}");
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

/// A held id that is no selected family's row at all: the C3 exemplar has to be
/// a page the identify, the challenge seam and the talk arm all leave alone, so
/// it is not a trails membership row, not one of the six challenge scrolls and
/// not a talk step.
fn first_unselected_id(data: &api::game_data::SelectedGameData) -> i32 {
    let trails = data.trails().expect("trails");
    let talk = data.talk_key().expect("talk_key");
    (1..)
        .find(|id| {
            !trails.rows.iter().any(|row| row.id == *id)
                && !trails.challenge_answers.iter().any(|row| row.id == *id)
                && !talk.talk.iter().any(|row| row.id == *id)
        })
        .expect("a held id no selected family names")
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
    // A held id no selected family names is not a membership row: the live step
    // is gone. It is neither a challenge scroll — the seam joins those onto
    // their parent talk step — nor a talk step of its own.
    let unselected = first_unselected_id(&data);
    let iso = LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data)
        .unwrap();
    post_page(&iso, 1, &[(2831, 1)]);
    iso.on_game_tick(1);
    post_page(&iso, 2, &[(unselected, 1)]);
    iso.on_game_tick(2);
    post_page(&iso, 3, &[(2831, 1)]);
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

/// One posted ground row: the same `SceneEntity` shape as a loc, plus the
/// display name the collect Take rides along with.
fn scene_ground<'a>(
    id: i32,
    name: &'a str,
    x: i32,
    z: i32,
    level: i32,
    actions: &'a [String],
) -> SceneEntityInput<'a> {
    let mut row = scene_loc(id, x, z, level, actions);
    row.name = Some(name);
    row
}

/// One posted npc row: the fields the guarded encounter reads — the posted
/// index the Attack carries, the posted display name the row family's
/// cap-documented wizard filter matches, the posted distance the frozen radius
/// reads, and the posted health pair the kill is read through. The posted
/// target pair is the default no-target pair unless a test sets it.
fn scene_npc<'a>(
    index: i32,
    name: &'a str,
    distance: i32,
    health: i32,
    max_health: i32,
    actions: &'a [String],
) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index,
        id: 100 + index,
        name: Some(name),
        x: 3058,
        z: 3884,
        level: 0,
        distance,
        health,
        max_health,
        in_combat: false,
        animating: false,
        actions,
        reachable: true,
        reachable_adj: true,
        combat_level: 0,
        target_kind: 0,
        target_index: -1,
        size: 1,
        nx: 3058,
        nz: 3884,
    }
}

/// One guarded fight call's keyframe: the posted `here`, the posted npc page,
/// the posted overlay varps, the posted stat rows and the posted cooperative
/// pair, over the pack page and display names the caller passes.
#[allow(clippy::too_many_arguments)]
fn guarded_scene<'a>(
    here: TileInput,
    names: &'a [(i32, &'a str)],
    npcs: &'a [SceneEntityInput<'a>],
    varps: &'a [VarpInput],
    stats: &'a [StatInput<'a>],
    ours: bool,
    hold: bool,
) -> Scene<'a> {
    Scene {
        here: Some(here),
        names,
        npcs,
        varps,
        stats,
        ours,
        hold,
        ..Scene::default()
    }
}

/// The `b` run's first piece (`trail_slidingpuzzleb01`), the run both pins
/// number 2749-2772: a cell's posted piece id is `PIECE_B + target`.
const PIECE_B: i32 = 2749;

/// `trail_clue_hard_riddle014`: the desc-only hard riddle whose own selected
/// `trail_clue_hard_riddle014_puzzlebox` item is the box its puzzle step joins
/// to.
const PUZZLE_RIDDLE: i32 = 2794;

/// That box item, display `Puzzle box`.
const PUZZLE_BOX: i32 = 2795;

/// `trail_clue_hard_riddle022`: a desc-only hard riddle with **no**
/// `_puzzlebox` sibling at all.
const PUZZLE_RIDDLE_NO_BOX: i32 = 3572;

/// `trail_clue_easy_simple001`: the selected search membership, decoded to
/// (3209, 3218, 1), which every walk scene below is posted far from.
const PUZZLE_SEARCH: i32 = 2677;

/// The component the test boards are posted at.
const BOARD_COMPONENT: i32 = 6600;

/// One posted board row: the piece's own obj id, the widget slot it sits in
/// and the component the page was identified at — the shape SNAP posts and the
/// plan reads.
fn board_row(id: i32, slot: i32, component: i32) -> script::isolate_fb::ItemRowInput<'static> {
    script::isolate_fb::ItemRowInput {
        name: Some("Sliding piece"),
        count: 1,
        id,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: component,
        slot,
    }
}

/// One posted board as its sparse rows: a row per filled cell, in slot order,
/// carrying the piece belonging on that cell's target. `PIECE_B` is the first
/// piece of the `b` run both pins hold, so the rows are the same on both.
fn board_rows(
    board: &[Option<u8>; 25],
    component: i32,
) -> Vec<script::isolate_fb::ItemRowInput<'static>> {
    let mut rows = Vec::new();
    for (slot, cell) in board.iter().enumerate() {
        if let Some(target) = *cell {
            rows.push(board_row(
                PIECE_B + i32::from(target),
                slot as i32,
                component,
            ));
        }
    }
    rows
}

/// The solved board: every piece on its own slot and the gap on 24.
fn solved_board() -> [Option<u8>; 25] {
    let mut board = [None; 25];
    for (slot, cell) in board.iter_mut().enumerate() {
        *cell = (slot != 24).then_some(slot as u8);
    }
    board
}

/// One slide from solved: the piece belonging on 23 stands on the blank slot,
/// so the frozen plan is the single click on 24.
fn one_move_board() -> [Option<u8>; 25] {
    let mut board = solved_board();
    board[24] = Some(23);
    board[23] = None;
    board
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

/// One unguarded Dig as the drain sees it: the selected item display name the
/// host resolves by first name match, and the frozen action. No row id and no
/// tile, exactly like the Open and the Drop it shares its kind with.
fn spade_dig() -> InteractReq {
    InteractReq::Held {
        name: "Spade".to_string(),
        action: "Dig".to_string(),
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

    // Tick 5: the casket gone, the constrained clue left alone. The live
    // session identifies 3554 and refuses it — `aborted` / `constrained`, no
    // verb, and the token dies with the refusal.
    post_page(&iso, 5, &[(3554, 1)]);
    iso.on_game_tick(5);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "3554 is a clue, not a casket: nothing is opened for it"
    );

    // Tick 6: the dead token hears `stale`, whatever the page holds.
    post_page(&iso, 6, &[]);
    iso.on_game_tick(6);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "a dead token pushes nothing"
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
    ] {
        assert_eq!(steps[index]["kind"], kind, "{index} {value:?}");
    }
    for index in 1..steps.len() - 2 {
        let step = &steps[index];
        assert_eq!(step["ok"], true, "{index} {step}");
        assert_eq!(step["status"], "continue", "{index} {step}");
        assert_eq!(step["token"], *token, "{index} {step}");
        assert!(step.get("error").is_none(), "{index} {step}");
    }
    // The constrained clue is the identify family's own refusal: no verb, no
    // continue kind, and the token dies with it.
    let constrained = &steps[8];
    assert_eq!(constrained["ok"], false, "{value:?}");
    assert_eq!(constrained["error"], "constrained", "{value:?}");
    assert!(constrained.get("value").is_none(), "{value:?}");
    assert_eq!(constrained["status"], serde_json::Value::Null, "{value:?}");
    // And the dead token hears `stale`, never a resumed step.
    let lost = &steps[9];
    assert_eq!(lost["ok"], false, "{value:?}");
    assert_eq!(lost["error"], "stale", "{value:?}");
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
/// fully walkable posted scene: 2831 is a desc-only frozen `keyFrom` riddle and
/// 2722 is a clue row with no params at all — and neither is a held casket, so
/// neither opens anything. The packed 3554 `access: "constrained"` clue is not
/// idled: it is refused, which is the sibling test below.
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
    for id in [2722, 2831] {
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

/// The packed 3554 `access: "constrained"` clue over the public path: the
/// machine refuses it — `clue.begin` answers `constrained` and leaves no token,
/// and a live session that identifies it answers the same and dies — with no
/// walk, no Dig, no Attack and no interact entry. Nothing invents a duel, a
/// partner or an arena for it.
#[test]
fn v2_clue_constrained_row_is_refused_and_never_played() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__runs = (globalThis.__runs || 0) + 1;
  if (globalThis.__runs === 1) {
    // The packed clue is the only held step: the begin is refused and hands
    // out no session at all.
    globalThis.__refused = api.clue.begin();
    return;
  }
  if (globalThis.__runs === 2) {
    // A paramless row is a real step, so the session below has a token.
    const begin = api.clue.begin();
    globalThis.__token = begin.ok ? begin.value.token : null;
    globalThis.__steps = [begin];
    return;
  }
  globalThis.__steps.push(api.clue.next(
    globalThis.__runs === 4
      ? { token: globalThis.__token, resume: true }
      : { token: globalThis.__token }));
  if (globalThis.__runs === 6) {
    globalThis.__probe = JSON.stringify({
      refused: globalThis.__refused,
      token: globalThis.__token,
      steps: globalThis.__steps,
    });
  }
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data)
        .unwrap();
    // A scene the search and dig arms would both play from: `here` on the
    // picked loc and the Spade in the pack.
    let actions = vec!["Search".to_string()];
    let locs = [scene_loc(25, 3209, 3218, 1, &actions)];
    let names = [(952, "Spade")];
    let scene = Scene {
        here: Some(TileInput {
            x: 3209,
            z: 3218,
            level: 1,
        }),
        locs: &locs,
        names: &names,
        ..Scene::default()
    };
    let packed = [(3554, 1), (952, 1)];
    let paramless = [(2722, 1), (952, 1)];

    post_scene(&iso, 1, &packed, &scene);
    iso.on_game_tick(1);
    assert!(iso.probe("true").is_ok());
    assert!(iso.drain_interacts().is_empty(), "the refusal is not a verb");

    for tick in 2..=4 {
        post_scene(&iso, tick, &paramless, &scene);
        iso.on_game_tick(tick);
        assert!(iso.probe("true").is_ok());
        assert!(
            iso.drain_interacts().is_empty(),
            "tick {tick}: the idle row pushes no verb"
        );
    }
    for tick in 5..=6 {
        post_scene(&iso, tick, &packed, &scene);
        iso.on_game_tick(tick);
        assert!(iso.probe("true").is_ok());
        assert!(
            iso.drain_interacts().is_empty(),
            "tick {tick}: a constrained row pushes no walk and no Dig"
        );
    }

    let probed = iso.probe("globalThis.__probe").unwrap();
    let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
    let interacts = iso.drain_interacts();
    iso.join();
    assert!(interacts.is_empty(), "{interacts:?}");

    // The begin refusal: the identify family's own shape, with no token.
    assert_eq!(value["refused"]["ok"], false, "{value:?}");
    assert_eq!(value["refused"]["error"], "constrained", "{value:?}");
    assert!(value["refused"].get("value").is_none(), "{value:?}");
    assert_eq!(
        value["refused"]["status"],
        serde_json::Value::Null,
        "{value:?}"
    );

    let steps = value["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 5, "{value:?}");
    for (index, kind) in [(1, "callback.enabled"), (2, "callback.log")] {
        assert_eq!(steps[index]["kind"], kind, "{index} {value:?}");
    }
    // The live session that identifies the packed clue: the same refusal, and
    // the token dies with it.
    assert_eq!(steps[3]["ok"], false, "{value:?}");
    assert_eq!(steps[3]["error"], "constrained", "{value:?}");
    assert!(steps[3].get("value").is_none(), "{value:?}");
    assert_eq!(steps[3]["status"], serde_json::Value::Null, "{value:?}");
    assert_eq!(steps[4]["ok"], false, "{value:?}");
    assert_eq!(steps[4]["error"], "stale", "{value:?}");
    let text = value.to_string();
    for forbidden in [
        "clue solved",
        "\"done\"",
        "abandon",
        "supplies-needed",
        "ownsEquipment",
        "duel",
        "arena",
    ] {
        assert!(!text.contains(forbidden), "{forbidden} {value:?}");
    }
}

/// The public unguarded-dig path: the held medium sextant clue walks to its
/// decoded `trail_coord` tile, then Digs with the Spade the same already-posted
/// `snapshot.inv` page carries — `InteractReq::Walk` then
/// `InteractReq::Held { name: "Spade", action: "Dig" }`, which no module test
/// can see. Dig repeats while the same clue stays held, the frozen/posted
/// interrupts emit no verb, and the casket the dig produced Opens instead.
#[test]
fn v2_clue_unguarded_dig_row_walks_then_digs_the_spade() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__runs = (globalThis.__runs || 0) + 1;
  if (globalThis.__runs === 1) {
    const begin = api.clue.begin();
    globalThis.__token = begin.ok ? begin.value.token : null;
    globalThis.__steps = [begin];
    return;
  }
  if (globalThis.__runs === 8) {
    // The Dig shape is already an author op: `held` is on V2_OPS and the
    // machine's own step is what goes through the interact drain.
    try {
      api.request({ op: 'held', name: 'Spade', action: 'Dig' });
      globalThis.__heldOp = 'ok';
    } catch (e) {
      globalThis.__heldOp = String(e && (e.message || e));
    }
  }
  globalThis.__steps.push(api.clue.next(
    globalThis.__runs === 3 || globalThis.__runs === 10
      ? { token: globalThis.__token, resume: true }
      : { token: globalThis.__token }));
  if (globalThis.__runs === 12) {
    globalThis.__probe = JSON.stringify({
      token: globalThis.__token,
      runs: globalThis.__runs,
      steps: globalThis.__steps,
      heldOp: globalThis.__heldOp,
      groundType: typeof api.snapshot.ground,
      locsType: typeof api.snapshot.locs,
    });
  }
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data)
        .unwrap();
    // `trail_clue_medium_sextant001` held beside the item its Dig resolves:
    // one page, because `snapshot.inv` is both the identify page and the pack
    // page the Spade is read from.
    let clue = [(2801, 1), (952, 1)];
    let produced = [(2801, 1), (2802, 1), (952, 1)];
    let names = [(952, "Spade")];
    let far = TileInput {
        x: 3100,
        z: 3300,
        level: 0,
    };
    let arrived = TileInput {
        x: 3160,
        z: 3251,
        level: 0,
    };
    let scene = |here: TileInput, ours: bool, hold: bool| Scene {
        here: Some(here),
        names: &names,
        ours,
        hold,
        ..Scene::default()
    };

    // Ticks 1-4: the begin and the landed report. Standing on the tile already
    // changes nothing ahead of `Steady`.
    for tick in 1..=4 {
        post_scene(&iso, tick, &clue, &scene(arrived, false, false));
        iso.on_game_tick(tick);
        assert!(iso.probe("true").is_ok());
        assert!(
            iso.drain_interacts().is_empty(),
            "tick {tick} pushes no verb"
        );
    }

    // Tick 5: not arrived, so the walk to the decoded 0_49_50_24_51 tile.
    post_scene(&iso, 5, &clue, &scene(far, false, false));
    iso.on_game_tick(5);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Walk {
            x: 3160,
            z: 3251,
            level: 0,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 0,
        }],
        "the walk is the decoded pin with the default flags"
    );

    // Ticks 6-7: arrived with the Spade posted: the generic held Dig, and it
    // repeats while the same clue stays held.
    for tick in 6..=7 {
        post_scene(&iso, tick, &clue, &scene(arrived, false, false));
        iso.on_game_tick(tick);
        assert!(iso.probe("true").is_ok());
        assert_eq!(
            iso.drain_interacts(),
            vec![spade_dig()],
            "tick {tick}: the Dig repeats while the clue is held"
        );
    }

    // Tick 8: the posted `hold` freezes this machine's clock and is a
    // paint-only tick — the script never runs — so the same arrived page with
    // the Spade reaches the drain with nothing.
    post_scene(&iso, 8, &clue, &scene(arrived, false, true));
    iso.on_game_tick(8);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "a frozen call emits no walk and no held"
    );
    assert_eq!(
        iso.probe("String(globalThis.__runs)").unwrap().as_str(),
        Some("7"),
        "the posted hold skipped the tick's script, and it skipped no verb"
    );

    // Tick 9: the posted `ours` interrupt, unfrozen: yield, and the only thing
    // on the drain is the probe's own `held` author op.
    post_scene(&iso, 9, &clue, &scene(arrived, true, false));
    iso.on_game_tick(9);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![spade_dig()],
        "yield emits no walk and no held of its own"
    );

    // Ticks 10-13: the dig produced its casket. Identify is casket-first, so
    // the landed gate re-arms for it and the Open follows — never a second Dig
    // and never a collect under a held step.
    for tick in 10..=12 {
        post_scene(&iso, tick, &produced, &scene(arrived, false, false));
        iso.on_game_tick(tick);
        assert!(iso.probe("true").is_ok());
        assert!(
            iso.drain_interacts().is_empty(),
            "tick {tick} pushes no verb"
        );
    }
    post_scene(&iso, 13, &produced, &scene(arrived, false, false));
    iso.on_game_tick(13);
    let probed = iso.probe("globalThis.__probe").unwrap();
    let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
    assert_eq!(
        iso.drain_interacts(),
        vec![casket_open()],
        "the produced casket is the Open, not 3554 play and not a Dig"
    );
    iso.join();

    assert_eq!(value["runs"], 12, "{value:?}");
    assert!(value["token"].is_number(), "{value:?}");
    let steps = value["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 12, "{value:?}");
    for (index, kind) in [
        (1, "callback.enabled"),
        (2, "callback.log"),
        (3, "callback.setStatus"),
        (4, "walk"),
        (5, "held"),
        (6, "held"),
        (7, "yield"),
        (8, "callback.enabled"),
        (9, "callback.log"),
        (10, "callback.setStatus"),
        (11, "held"),
    ] {
        assert_eq!(steps[index]["kind"], kind, "{index} {value:?}");
    }
    for index in 1..steps.len() {
        let step = &steps[index];
        assert_eq!(step["ok"], true, "{index} {step}");
        assert_eq!(step["status"], "continue", "{index} {step}");
        assert_eq!(step["token"], value["token"], "{index} {step}");
        assert!(step.get("error").is_none(), "{index} {step}");
    }
    // The decode pin on the public path: 2801's selected 0_49_50_24_51 token
    // is (3160, 3251, 0).
    assert_eq!(steps[4]["x"], 3160, "{value:?}");
    assert_eq!(steps[4]["z"], 3251, "{value:?}");
    assert_eq!(steps[4]["level"], 0, "{value:?}");
    // The Dig is the selected display and the frozen action, with no bound row
    // id and no tile.
    for index in [5, 6] {
        assert_eq!(steps[index]["name"], "Spade", "{index} {value:?}");
        assert_eq!(steps[index]["action"], "Dig", "{index} {value:?}");
        for absent in ["id", "x", "z", "level", "message"] {
            assert!(
                steps[index].get(absent).is_none(),
                "{index} {absent} {value:?}"
            );
        }
    }
    // The produced casket's own report and its own Open.
    assert_eq!(steps[11]["name"], "Casket", "{value:?}");
    assert_eq!(steps[11]["action"], "Open", "{value:?}");
    let report = steps[2]["message"].as_str().unwrap_or("");
    assert!(report.contains("trail_clue_medium_sextant001"), "{value:?}");
    assert!(report.contains("2801"), "{value:?}");
    let casket_report = steps[9]["message"].as_str().unwrap_or("");
    assert!(
        casket_report.contains("trail_clue_medium_sextant001_casket"),
        "{value:?}"
    );
    // No completion, no abandonment, and no no-spade token.
    let text = value.to_string();
    for forbidden in [
        "clue solved",
        "abandon",
        "supplies-needed",
        "no-spade",
        "ownsEquipment",
        "\"done\"",
    ] {
        assert!(!text.contains(forbidden), "{forbidden} {value:?}");
    }
    // `held` stays an author op and the unpublished snapshot pages stay hidden.
    assert_eq!(value["heldOp"], "ok", "{value:?}");
    for key in ["groundType", "locsType"] {
        assert_eq!(value[key], "undefined", "{key} {value:?}");
    }
}

/// The public no-Spade path: arrived on the decoded tile with a pack that does
/// not post the Spade is the named `supplies-needed` wait-class — the token
/// stays live, nothing reaches the drain, and no token-killing `no-spade` error
/// is published. The Dig is still there once the pack carries it.
#[test]
fn v2_clue_unguarded_dig_row_waits_without_the_spade() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__runs = (globalThis.__runs || 0) + 1;
  if (globalThis.__runs === 1) {
    const begin = api.clue.begin();
    globalThis.__token = begin.ok ? begin.value.token : null;
    globalThis.__steps = [begin];
    return;
  }
  globalThis.__steps.push(api.clue.next(
    globalThis.__runs === 3
      ? { token: globalThis.__token, resume: true }
      : { token: globalThis.__token }));
  if (globalThis.__runs === 6) {
    globalThis.__probe = JSON.stringify({
      token: globalThis.__token,
      steps: globalThis.__steps,
    });
  }
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data)
        .unwrap();
    // The held clue alone: the Spade never reached the pack.
    let bare = [(2801, 1)];
    let with_spade = [(2801, 1), (952, 1)];
    let names = [(952, "Spade")];
    let arrived = TileInput {
        x: 3160,
        z: 3251,
        level: 0,
    };
    let scene = |ours: bool| Scene {
        here: Some(arrived),
        names: &names,
        ours,
        ..Scene::default()
    };

    for tick in 1..=5 {
        post_scene(&iso, tick, &bare, &scene(false));
        iso.on_game_tick(tick);
        assert!(iso.probe("true").is_ok());
        assert!(
            iso.drain_interacts().is_empty(),
            "tick {tick}: a missing Spade is a wait, not a verb"
        );
    }
    // The same live token, and the pack that posts the Spade is the Dig.
    post_scene(&iso, 6, &with_spade, &scene(false));
    iso.on_game_tick(6);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![spade_dig()],
        "the Spade is the verb"
    );
    // The posted `ours` interrupt still beats it, with the Spade in hand.
    post_scene(&iso, 7, &with_spade, &scene(true));
    iso.on_game_tick(7);
    assert!(iso.probe("true").is_ok());
    assert!(iso.drain_interacts().is_empty(), "yield emits no held Dig");

    let probed = iso.probe("globalThis.__probe").unwrap();
    let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
    let steps: serde_json::Value = serde_json::from_str(
        iso.probe("JSON.stringify(globalThis.__steps)")
            .unwrap()
            .as_str()
            .unwrap(),
    )
    .unwrap();
    iso.join();
    let steps = steps.as_array().expect("steps");
    assert_eq!(steps.len(), 7, "{value:?}");
    // The reported step that arrived without the Spade is the named
    // wait-class, on the live token that Digs once the pack posts it.
    assert_eq!(steps[4]["kind"], "supplies-needed", "{value:?}");
    assert_eq!(steps[4]["ok"], true, "{value:?}");
    assert_eq!(steps[4]["status"], "continue", "{value:?}");
    assert_eq!(steps[4]["token"], value["token"], "{value:?}");
    for absent in ["name", "action", "x", "z", "level", "message", "id", "error"] {
        assert!(steps[4].get(absent).is_none(), "{absent} {value:?}");
    }
    assert_eq!(steps[5]["kind"], "held", "{value:?}");
    assert_eq!(steps[5]["token"], value["token"], "{value:?}");
    assert_eq!(steps[6]["kind"], "yield", "{value:?}");
    let text = value.to_string();
    for forbidden in ["no-spade", "abandon", "\"done\"", "clue solved", "dead"] {
        assert!(!text.contains(forbidden), "{forbidden} {value:?}");
    }
}

/// The public coord-only map path: `trail_clue_easy_map001` (`2713`,
/// `0_49_52_41_32`) carries no `trail_sextant` and is still the widened dig
/// membership, so it walks to its decoded (3177, 3360, 0) and then Digs with
/// the Spade the already-posted `snapshot.inv` page carries —
/// `InteractReq::Walk` then `InteractReq::Held { name: "Spade", action: "Dig" }`
/// — and the easy casket that Dig produces is the landed casket-first Open.
/// The guarded row is not one of the idle rows — its own encounter walks and
/// Digs from this same scene.
#[test]
fn v2_clue_coord_only_map_row_walks_then_digs_the_spade() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__runs = (globalThis.__runs || 0) + 1;
  if (globalThis.__runs === 1) {
    const begin = api.clue.begin();
    globalThis.__token = begin.ok ? begin.value.token : null;
    globalThis.__steps = [begin];
    return;
  }
  globalThis.__steps.push(api.clue.next(
    globalThis.__runs === 3 || globalThis.__runs === 9
      ? { token: globalThis.__token, resume: true }
      : { token: globalThis.__token }));
  if (globalThis.__runs === 11) {
    globalThis.__probe = JSON.stringify({
      token: globalThis.__token,
      runs: globalThis.__runs,
      steps: globalThis.__steps,
      groundType: typeof api.snapshot.ground,
      locsType: typeof api.snapshot.locs,
    });
  }
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data)
        .unwrap();
    // The map row held beside the Spade its Dig resolves: one page, because
    // `snapshot.inv` is both the identify page and the pack page.
    let clue = [(2713, 1), (952, 1)];
    let produced = [(2713, 1), (2714, 1), (952, 1)];
    let names = [(952, "Spade")];
    let far = TileInput {
        x: 3100,
        z: 3300,
        level: 0,
    };
    let arrived = TileInput {
        x: 3177,
        z: 3360,
        level: 0,
    };
    let scene = |here: TileInput| Scene {
        here: Some(here),
        names: &names,
        ..Scene::default()
    };

    // Ticks 1-4: the begin and the landed report. Standing on the tile already
    // changes nothing ahead of `Steady`.
    for tick in 1..=4 {
        post_scene(&iso, tick, &clue, &scene(arrived));
        iso.on_game_tick(tick);
        assert!(iso.probe("true").is_ok());
        assert!(
            iso.drain_interacts().is_empty(),
            "tick {tick} pushes no verb"
        );
    }

    // Tick 5: not arrived, so the walk to the map's own decoded tile.
    post_scene(&iso, 5, &clue, &scene(far));
    iso.on_game_tick(5);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Walk {
            x: 3177,
            z: 3360,
            level: 0,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 0,
        }],
        "the walk is the map's decoded pin"
    );

    // Ticks 6-7: arrived with the Spade posted: the generic held Dig, and it
    // repeats while the same clue stays held.
    for tick in 6..=7 {
        post_scene(&iso, tick, &clue, &scene(arrived));
        iso.on_game_tick(tick);
        assert!(iso.probe("true").is_ok());
        assert_eq!(
            iso.drain_interacts(),
            vec![spade_dig()],
            "tick {tick}: the Dig repeats while the clue is held"
        );
    }

    // Ticks 8-10: the dig produced its easy casket. Identify is casket-first,
    // so the landed gate re-arms for it and its own Open follows.
    for tick in 8..=10 {
        post_scene(&iso, tick, &produced, &scene(arrived));
        iso.on_game_tick(tick);
        assert!(iso.probe("true").is_ok());
        assert!(
            iso.drain_interacts().is_empty(),
            "tick {tick} pushes no verb"
        );
    }
    post_scene(&iso, 11, &produced, &scene(arrived));
    iso.on_game_tick(11);
    let probed = iso.probe("globalThis.__probe").unwrap();
    let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
    assert_eq!(
        iso.drain_interacts(),
        vec![casket_open()],
        "the produced casket is the Open, not a Dig"
    );
    iso.join();

    assert_eq!(value["runs"], 11, "{value:?}");
    assert!(value["token"].is_number(), "{value:?}");
    let steps = value["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 11, "{value:?}");
    for (index, kind) in [
        (1, "callback.enabled"),
        (2, "callback.log"),
        (3, "callback.setStatus"),
        (4, "walk"),
        (5, "held"),
        (6, "held"),
        (7, "callback.enabled"),
        (8, "callback.log"),
        (9, "callback.setStatus"),
        (10, "held"),
    ] {
        assert_eq!(steps[index]["kind"], kind, "{index} {value:?}");
    }
    for index in 1..steps.len() {
        let step = &steps[index];
        assert_eq!(step["ok"], true, "{index} {step}");
        assert_eq!(step["status"], "continue", "{index} {step}");
        assert_eq!(step["token"], value["token"], "{index} {step}");
        assert!(step.get("error").is_none(), "{index} {step}");
    }
    // The decode pin on the public path: 2713's selected 0_49_52_41_32 token
    // is (3177, 3360, 0).
    assert_eq!(steps[4]["x"], 3177, "{value:?}");
    assert_eq!(steps[4]["z"], 3360, "{value:?}");
    assert_eq!(steps[4]["level"], 0, "{value:?}");
    // The Dig is the selected display and the frozen action, with no bound row
    // id and no tile.
    for index in [5, 6] {
        assert_eq!(steps[index]["name"], "Spade", "{index} {value:?}");
        assert_eq!(steps[index]["action"], "Dig", "{index} {value:?}");
        for absent in ["id", "x", "z", "level", "message"] {
            assert!(
                steps[index].get(absent).is_none(),
                "{index} {absent} {value:?}"
            );
        }
    }
    // The produced easy casket's own report and its own Open.
    assert_eq!(steps[10]["name"], "Casket", "{value:?}");
    assert_eq!(steps[10]["action"], "Open", "{value:?}");
    let report = steps[2]["message"].as_str().unwrap_or("");
    assert!(report.contains("trail_clue_easy_map001"), "{value:?}");
    assert!(report.contains("2713"), "{value:?}");
    let casket_report = steps[8]["message"].as_str().unwrap_or("");
    assert!(
        casket_report.contains("trail_clue_easy_map001_casket"),
        "{value:?}"
    );
    // No completion, no abandonment, and no no-spade token.
    let text = value.to_string();
    for forbidden in [
        "clue solved",
        "abandon",
        "supplies-needed",
        "no-spade",
        "ownsEquipment",
        "\"done\"",
    ] {
        assert!(!text.contains(forbidden), "{forbidden} {value:?}");
    }
    // The unpublished snapshot pages stay hidden.
    for key in ["groundType", "locsType"] {
        assert_eq!(value[key], "undefined", "{key} {value:?}");
    }
}

/// The public guarded encounter over the posted page and scene: the exemplar
/// `trail_clue_hard_sextant001` (`2723`, `0_47_60_50_44`) walks to its decoded
/// tile, Digs its spawn, raises Protect from Magic with the landed generic
/// `if-button` while the posted overlay is off, Attacks the posted wizard once
/// the overlay is up, waits while it is still posted, and — once that index
/// leaves the page inside the frozen grace — walks back and Digs again. The
/// frozen `hold` and the posted `ours` still beat all of it.
#[test]
fn v2_clue_guarded_row_walks_digs_attacks_and_redigs() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__runs = (globalThis.__runs || 0) + 1;
  if (globalThis.__runs === 1) {
    const begin = api.clue.begin();
    globalThis.__token = begin.ok ? begin.value.token : null;
    globalThis.__steps = [begin];
    return;
  }
  globalThis.__steps.push(api.clue.next(
    globalThis.__runs === 3
      ? { token: globalThis.__token, resume: true }
      : { token: globalThis.__token }));
  if (globalThis.__runs === 12) {
    // The machine's own `npc` and `if-button` steps are not author ops: both
    // stay off V2_OPS while the machine enqueues them onto the drain.
    let npcOp = null;
    try {
      api.request({ op: 'npc', name: 'Zamorak Wizard', action: 'Attack', index: 7 });
      npcOp = 'ok';
    } catch (e) { npcOp = String(e && (e.message || e)); }
    let ifOp = null;
    try { api.request({ op: 'if-button', component_id: 5621 }); ifOp = 'ok'; }
    catch (e) { ifOp = String(e && (e.message || e)); }
    globalThis.__probe = JSON.stringify({
      token: globalThis.__token,
      steps: globalThis.__steps,
      npcOp,
      ifOp,
      groundType: typeof api.snapshot.ground,
      locsType: typeof api.snapshot.locs,
      npcsType: typeof api.snapshot.npcs,
      selfSlotType: typeof api.snapshot.self_slot,
      varpsType: typeof api.snapshot.varps,
    });
  }
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data)
        .unwrap();
    // `trail_clue_hard_sextant001` beside the item its Dig resolves, and the
    // posted pages the fight reads: the npc page, the overlay varps (the
    // selected Protect from Magic index 95, off then on) and the posted
    // `hitpoints` the mid-fight wait reads.
    let page = [(2723, 1), (952, 1)];
    let names = [(952, "Spade")];
    let attack = vec!["Attack".to_string()];
    let wizard = scene_npc(7, "Zamorak Wizard", 3, 10, 10, &attack);
    let stats = [StatInput {
        index: 3,
        name: "hitpoints",
        xp: 0,
        base: 40,
        effective: 40,
    }];
    let overlay_off = [VarpInput { index: 95, value: 0 }];
    let overlay_on = [VarpInput { index: 95, value: 1 }];
    let far = TileInput {
        x: 3100,
        z: 3300,
        level: 0,
    };
    let arrived = TileInput {
        x: 3058,
        z: 3884,
        level: 0,
    };
    // Ticks 1-4: the begin and the landed report. Nothing on the drain yet.
    for tick in 1..=4 {
        post_scene(&iso, tick, &page, &guarded_scene(far, &names, &[], &[], &stats, false, false));
        iso.on_game_tick(tick);
        assert!(iso.probe("true").is_ok());
        assert!(
            iso.drain_interacts().is_empty(),
            "tick {tick} pushes no verb"
        );
    }

    // Tick 5: not arrived, so the walk to the decoded `0_47_60_50_44` tile.
    post_scene(&iso, 5, &page, &guarded_scene(far, &names, &[], &[], &stats, false, false));
    iso.on_game_tick(5);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Walk {
            x: 3058,
            z: 3884,
            level: 0,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 0,
        }],
        "the guarded row walks to its own selected pin"
    );

    // Tick 6: arrived with the Spade: the guarded first Dig, which is the
    // spawn. No npc page and no overlay have been posted yet.
    post_scene(&iso, 6, &page, &guarded_scene(arrived, &names, &[], &[], &stats, false, false));
    iso.on_game_tick(6);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![spade_dig()],
        "the first Dig is the spawn, and it is the landed Spade Dig"
    );

    // Tick 7: the wizard is posted and the overlay reads off: the landed
    // generic `if-button` with the selected component, and no Attack.
    post_scene(
        &iso,
        7,
        &page,
        &guarded_scene(arrived, &names, &[wizard], &overlay_off, &stats, false, false),
    );
    iso.on_game_tick(7);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::IfButton { component_id: 5621 }],
        "the overlay off is the click and never an Attack"
    );

    // Tick 8: the overlay posted on: the Attack, at the posted index.
    post_scene(
        &iso,
        8,
        &page,
        &guarded_scene(arrived, &names, &[wizard], &overlay_on, &stats, false, false),
    );
    iso.on_game_tick(8);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Npc {
            name: "Zamorak Wizard".to_string(),
            action: "Attack".to_string(),
            index: Some(7),
        }],
        "the Attack is the posted name, action and index"
    );

    // Tick 9: still posted and alive: the fight waits for its kill.
    post_scene(
        &iso,
        9,
        &page,
        &guarded_scene(arrived, &names, &[wizard], &overlay_on, &stats, false, false),
    );
    iso.on_game_tick(9);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "a posted, living wizard is a wait"
    );

    // Tick 10: the owned index left the page inside the grace — the kill. The
    // player is off the tile here, so the kill walks back.
    post_scene(&iso, 10, &page, &guarded_scene(far, &names, &[], &overlay_on, &stats, false, false));
    iso.on_game_tick(10);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Walk {
            x: 3058,
            z: 3884,
            level: 0,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 0,
        }],
        "the kill walks back to the decoded pin"
    );

    // Tick 11: arrived again: the post-kill redig.
    post_scene(&iso, 11, &page, &guarded_scene(arrived, &names, &[], &overlay_on, &stats, false, false));
    iso.on_game_tick(11);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![spade_dig()],
        "the post-kill Dig is the landed Spade Dig"
    );

    // Tick 12: the posted `hold` freezes this machine's clock and is a
    // paint-only tick — the script never runs — so the same arrived page with
    // the Spade reaches the drain with nothing.
    post_scene(&iso, 12, &page, &guarded_scene(arrived, &names, &[], &overlay_on, &stats, false, true));
    iso.on_game_tick(12);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "a frozen call emits no walk, no held, no npc and no if-button"
    );

    // Tick 13: the posted `ours` interrupt, unfrozen: yield, and the drain is
    // empty again.
    post_scene(&iso, 13, &page, &guarded_scene(arrived, &names, &[], &overlay_on, &stats, true, false));
    iso.on_game_tick(13);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "yield emits no verb of its own"
    );

    let probed = iso.probe("globalThis.__probe").unwrap();
    let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
    iso.join();

    let steps = value["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 12, "{value:?}");
    for (index, kind) in [
        (1, "callback.enabled"),
        (2, "callback.log"),
        (3, "callback.setStatus"),
        (4, "walk"),
        (5, "held"),
        (6, "if-button"),
        (7, "npc"),
        (8, "wait"),
        (9, "walk"),
        (10, "held"),
        (11, "yield"),
    ] {
        assert_eq!(steps[index]["kind"], kind, "{index} {value:?}");
    }
    for index in 1..steps.len() {
        let step = &steps[index];
        assert_eq!(step["ok"], true, "{index} {step}");
        assert_eq!(step["status"], "continue", "{index} {step}");
        assert_eq!(step["token"], value["token"], "{index} {step}");
        assert!(step.get("error").is_none(), "{index} {step}");
    }
    // The decode pin on the public path: 2723's selected token is the guarded
    // exemplar's own (3058, 3884, 0).
    assert_eq!(steps[4]["x"], 3058, "{value:?}");
    assert_eq!(steps[4]["z"], 3884, "{value:?}");
    assert_eq!(steps[4]["level"], 0, "{value:?}");
    // The spawn Dig and the redig are the selected display and the frozen
    // action, with no row id and no tile.
    for index in [5, 10] {
        assert_eq!(steps[index]["name"], "Spade", "{index} {value:?}");
        assert_eq!(steps[index]["action"], "Dig", "{index} {value:?}");
        for absent in ["id", "x", "z", "level", "message"] {
            assert!(
                steps[index].get(absent).is_none(),
                "{index} {absent} {value:?}"
            );
        }
    }
    // The click is the selected Protect from Magic component and carries
    // nothing else; the Attack is the posted identity.
    assert_eq!(steps[6]["component_id"], 5621, "{value:?}");
    assert!(steps[6].get("action").is_none(), "{value:?}");
    assert_eq!(steps[7]["name"], "Zamorak Wizard", "{value:?}");
    assert_eq!(steps[7]["action"], "Attack", "{value:?}");
    assert_eq!(steps[7]["index"], 7, "{value:?}");
    let report = steps[2]["message"].as_str().unwrap_or("");
    assert!(report.contains("trail_clue_hard_sextant001"), "{value:?}");
    assert!(report.contains("2723"), "{value:?}");
    // No completion, no abandonment, and none of the refusal tokens a later
    // slice owns.
    let text = value.to_string();
    for forbidden in [
        "clue solved",
        "abandon",
        "supplies-needed",
        "dead",
        "guardian-lost",
        "no-spade",
        "ownsEquipment",
        "\"done\"",
    ] {
        assert!(!text.contains(forbidden), "{forbidden} {value:?}");
    }
    // The machine's own npc and if-button steps stay off V2_OPS, and the
    // hidden snapshot pages stay hidden — including the slot and varps this
    // marshal reads straight from `host().snapshot`.
    assert_eq!(value["npcOp"], "not impl: request.npc", "{value:?}");
    assert_eq!(value["ifOp"], "not impl: request.if-button", "{value:?}");
    for key in ["groundType", "locsType", "selfSlotType", "varpsType"] {
        assert_eq!(value[key], "undefined", "{key} {value:?}");
    }
    assert_eq!(value["npcsType"], "object", "{value:?}");
}

/// The guarded spawn wait and a wizard that leaves before any Attack: a posted
/// page with no wizard of the row family waits — another name, and the nearest
/// anything, is never Attacked, and the overlay is never raised for a spawn
/// that was not posted — and a disappearance under an overlay that is still
/// off is the same wait, never a redig and never a `guardian-lost`.
#[test]
fn v2_clue_guarded_row_waits_without_an_attacked_wizard() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__runs = (globalThis.__runs || 0) + 1;
  if (globalThis.__runs === 1) {
    const begin = api.clue.begin();
    globalThis.__token = begin.ok ? begin.value.token : null;
    globalThis.__steps = [begin];
    return;
  }
  globalThis.__steps.push(api.clue.next(
    globalThis.__runs === 3
      ? { token: globalThis.__token, resume: true }
      : { token: globalThis.__token }));
  if (globalThis.__runs === 10) {
    globalThis.__probe = JSON.stringify({
      token: globalThis.__token,
      steps: globalThis.__steps,
    });
  }
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data)
        .unwrap();
    let page = [(2723, 1), (952, 1)];
    let names = [(952, "Spade")];
    let attack = vec!["Attack".to_string()];
    let other = vec!["Pick".to_string()];
    let guard = scene_npc(7, "Guard", 1, 10, 10, &attack);
    let wizard = scene_npc(9, "Zamorak Wizard", 3, 10, 10, &attack);
    let bystander = scene_npc(11, "Zamorak Wizard", 3, 10, 10, &other);
    let overlay_off = [VarpInput { index: 95, value: 0 }];
    let overlay_on = [VarpInput { index: 95, value: 1 }];
    let arrived = TileInput {
        x: 3058,
        z: 3884,
        level: 0,
    };
    // Ticks 1-4: the begin and the landed report.
    for tick in 1..=4 {
        post_scene(
            &iso,
            tick,
            &page,
            &guarded_scene(arrived, &names, &[], &overlay_off, &[], false, false),
        );
        iso.on_game_tick(tick);
        assert!(iso.probe("true").is_ok());
        assert!(iso.drain_interacts().is_empty(), "tick {tick}");
    }

    // Tick 5: arrived with the Spade: the spawn.
    post_scene(
        &iso,
        5,
        &page,
        &guarded_scene(arrived, &names, &[], &overlay_off, &[], false, false),
    );
    iso.on_game_tick(5);
    assert!(iso.probe("true").is_ok());
    assert_eq!(iso.drain_interacts(), vec![spade_dig()]);

    // Tick 6: the overlay is up and no wizard is posted at all: the spawn
    // wait, and nothing on the drain.
    post_scene(
        &iso,
        6,
        &page,
        &guarded_scene(arrived, &names, &[], &overlay_on, &[], false, false),
    );
    iso.on_game_tick(6);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "an empty npc page is the spawn wait"
    );

    // Tick 7: another name at distance one, and the right name without the
    // Attack action: neither is the nearest anything.
    post_scene(&iso, 7, &page, &guarded_scene(arrived, &names, &[guard, bystander], &overlay_on, &[], false, false));
    iso.on_game_tick(7);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "the nearest anything is never Attacked"
    );

    // Tick 8: the right name and action with the overlay off: the click, and
    // still no Attack.
    post_scene(&iso, 8, &page, &guarded_scene(arrived, &names, &[wizard], &overlay_off, &[], false, false));
    iso.on_game_tick(8);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::IfButton { component_id: 5621 }]
    );

    // Tick 9: the wizard left while nothing was ever Attacked: no spawn is
    // posted, so the overlay is not raised either — a wait, and never a
    // redig, a walk or a `guardian-lost`.
    post_scene(&iso, 9, &page, &guarded_scene(arrived, &names, &[], &overlay_off, &[], false, false));
    iso.on_game_tick(9);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "an unowned disappearance is the spawn wait and not a redig"
    );

    // Tick 10: the probe. The token is still live, the encounter never turned
    // into a Dig, and a page with no wizard of the family never clicks.
    post_scene(&iso, 10, &page, &guarded_scene(arrived, &names, &[], &overlay_off, &[], false, false));
    iso.on_game_tick(10);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "the encounter never turned into a Dig and never clicked for a spawn it cannot see"
    );

    let probed = iso.probe("globalThis.__probe").unwrap();
    let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
    iso.join();
    let steps = value["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 10, "{value:?}");
    for (index, kind) in [
        (1, "callback.enabled"),
        (2, "callback.log"),
        (3, "callback.setStatus"),
        (4, "held"),
        (5, "wait"),
        (6, "wait"),
        (7, "if-button"),
        (8, "wait"),
        (9, "wait"),
    ] {
        assert_eq!(steps[index]["kind"], kind, "{index} {value:?}");
    }
    for step in &steps[5..] {
        assert_eq!(step["token"], value["token"], "{step}");
        assert!(step.get("action").is_none(), "{step}");
    }
    let text = value.to_string();
    for forbidden in [
        "clue solved",
        "abandon",
        "supplies-needed",
        "dead",
        "guardian-lost",
        "\"done\"",
    ] {
        assert!(!text.contains(forbidden), "{forbidden} {value:?}");
    }
}

/// The guarded fight's marshalled npc page, driven through the public path: the
/// wrapper hands the machine the posted index, id, name, tile, distance,
/// health pair, `in_combat` flag, actions and target pair, and the spawn filter
/// reads that tile. A wizard of the row family on another level is not this
/// Dig's spawn; a row that posted no distance at all is measured against the
/// posted `here` with the landed Chebyshev read, inside the frozen radius 12 —
/// and the overlay click is only ever raised behind such a posted spawn.
#[test]
fn v2_clue_guarded_spawn_reads_the_marshalled_npc_page() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__runs = (globalThis.__runs || 0) + 1;
  if (globalThis.__runs === 1) {
    const begin = api.clue.begin();
    globalThis.__token = begin.ok ? begin.value.token : null;
    globalThis.__steps = [begin];
    return;
  }
  globalThis.__steps.push(api.clue.next(
    globalThis.__runs === 3
      ? { token: globalThis.__token, resume: true }
      : { token: globalThis.__token }));
  if (globalThis.__runs === 10) {
    globalThis.__probe = JSON.stringify({
      token: globalThis.__token,
      steps: globalThis.__steps,
    });
  }
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data)
        .unwrap();
    let page = [(2723, 1), (952, 1)];
    let names = [(952, "Spade")];
    let attack = vec!["Attack".to_string()];
    // The posted wizard one level up: the row's own posted level is part of
    // the spawn membership.
    let elsewhere = SceneEntityInput {
        level: 1,
        ..scene_npc(7, "Zamorak Wizard", 3, 10, 10, &attack)
    };
    let overlay_off = [VarpInput { index: 95, value: 0 }];
    let overlay_on = [VarpInput { index: 95, value: 1 }];
    let arrived = TileInput {
        x: 3058,
        z: 3884,
        level: 0,
    };
    // The same wrapper marshal over a raw posted page, where the row posts no
    // distance at all: its own `x`/`z`/`level` tile is the measure, and the
    // posted `id`/`in_combat` ride along. Two tiles out and thirteen tiles out
    // are the near and far halves of the frozen radius.
    let near = "globalThis.__rs2b0t_host.snapshot.npcs = [{ index: 7, id: 107, \
         name: 'Zamorak Wizard', x: 3058, z: 3886, level: 0, health: 10, \
         max_health: 10, in_combat: true, actions: ['Attack'], \
         target_kind: 0, target_index: -1 }]; true";
    let outside = "globalThis.__rs2b0t_host.snapshot.npcs = [{ index: 7, id: 107, \
         name: 'Zamorak Wizard', x: 3058, z: 3897, level: 0, health: 10, \
         max_health: 10, in_combat: true, actions: ['Attack'], \
         target_kind: 0, target_index: -1 }]; true";

    // Ticks 1-4: the begin and the landed report, already arrived with the
    // Spade and with no npc page posted at all.
    for tick in 1..=4 {
        post_scene(
            &iso,
            tick,
            &page,
            &guarded_scene(arrived, &names, &[], &overlay_off, &[], false, false),
        );
        iso.on_game_tick(tick);
        assert!(iso.probe("true").is_ok());
    }
    assert!(
        iso.drain_interacts().is_empty(),
        "the landed report pushes no verb"
    );

    // Tick 5: arrived with the Spade: the first Dig, which is the spawn.
    post_scene(
        &iso,
        5,
        &page,
        &guarded_scene(arrived, &names, &[], &overlay_off, &[], false, false),
    );
    iso.on_game_tick(5);
    assert!(iso.probe("true").is_ok());
    assert_eq!(iso.drain_interacts(), vec![spade_dig()], "the spawn Dig");

    // Tick 6: the wizard is posted on another level: not this Dig's spawn, so
    // the fight waits with no click and no Attack.
    post_scene(
        &iso,
        6,
        &page,
        &guarded_scene(arrived, &names, &[elsewhere], &overlay_off, &[], false, false),
    );
    iso.on_game_tick(6);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "another level is not the spawn this Dig made"
    );

    // Tick 7: the marshalled row thirteen tiles out with no posted distance:
    // outside the frozen radius, so the spawn wait and never a click.
    post_scene(
        &iso,
        7,
        &page,
        &guarded_scene(arrived, &names, &[], &overlay_off, &[], false, false),
    );
    iso.probe(outside).unwrap();
    iso.on_game_tick(7);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "the Chebyshev radius still refuses the row"
    );

    // Tick 8: the same row two tiles out with the overlay off: the spawn is
    // observed, so the click goes out and the Attack does not.
    post_scene(
        &iso,
        8,
        &page,
        &guarded_scene(arrived, &names, &[], &overlay_off, &[], false, false),
    );
    iso.probe(near).unwrap();
    iso.on_game_tick(8);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::IfButton { component_id: 5621 }],
        "the click is only raised behind a posted spawn"
    );

    // Tick 9: the same row, same tile, overlay on: the Attack at the posted
    // index.
    post_scene(
        &iso,
        9,
        &page,
        &guarded_scene(arrived, &names, &[], &overlay_on, &[], false, false),
    );
    iso.probe(near).unwrap();
    iso.on_game_tick(9);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Npc {
            name: "Zamorak Wizard".to_string(),
            action: "Attack".to_string(),
            index: Some(7),
        }],
        "the tile-measured spawn is the Attack"
    );

    // Tick 10: the probe. The owned wizard is still posted and alive, so the
    // fight waits for its kill and the Attack is not re-issued.
    post_scene(
        &iso,
        10,
        &page,
        &guarded_scene(arrived, &names, &[], &overlay_on, &[], false, false),
    );
    iso.probe(near).unwrap();
    iso.on_game_tick(10);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "the owned wizard is posted and alive"
    );

    let probed = iso.probe("globalThis.__probe").unwrap();
    let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
    iso.join();
    let steps = value["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 10, "{value:?}");
    for (index, kind) in [
        (1, "callback.enabled"),
        (2, "callback.log"),
        (3, "callback.setStatus"),
        (4, "held"),
        (5, "wait"),
        (6, "wait"),
        (7, "if-button"),
        (8, "npc"),
        (9, "wait"),
    ] {
        assert_eq!(steps[index]["kind"], kind, "{index} {value:?}");
    }
    assert_eq!(steps[4]["name"], "Spade", "{value:?}");
    assert_eq!(steps[4]["action"], "Dig", "{value:?}");
    assert_eq!(steps[7]["component_id"], 5621, "{value:?}");
    assert_eq!(steps[8]["name"], "Zamorak Wizard", "{value:?}");
    assert_eq!(steps[8]["action"], "Attack", "{value:?}");
    assert_eq!(steps[8]["index"], 7, "{value:?}");
    for index in 1..steps.len() {
        assert_eq!(steps[index]["token"], value["token"], "{index} {value:?}");
        assert!(steps[index].get("error").is_none(), "{index} {value:?}");
    }
    let text = value.to_string();
    for forbidden in [
        "clue solved",
        "abandon",
        "supplies-needed",
        "dead",
        "guardian-lost",
        "\"done\"",
    ] {
        assert!(!text.contains(forbidden), "{forbidden} {value:?}");
    }
}

/// The public `dead` terminal: any live call whose posted effective
/// `hitpoints` is at or below zero ends the session — no verb, no reason and
/// never `'clue solved'` — and a page that posted no stat at all is not a
/// zero.
#[test]
fn v2_clue_posted_hitpoints_at_zero_is_dead() {
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
    ];
    return;
  }
  globalThis.__steps.push(api.clue.next({ token: globalThis.__token }));
  if (globalThis.__runs === 5) {
    globalThis.__probe = JSON.stringify({
      token: globalThis.__token,
      steps: globalThis.__steps,
    });
  }
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data)
        .unwrap();
    let page = [(2677, 1)];
    let downed = [StatInput {
        index: 3,
        name: "hitpoints",
        xp: 0,
        base: 40,
        effective: 0,
    }];
    let healthy = [StatInput {
        index: 3,
        name: "hitpoints",
        xp: 0,
        base: 40,
        effective: 40,
    }];

    // Tick 1: the session opens over the held row and idles — no posted
    // `here`, so there is no arrival claim and no verb.
    post_page(&iso, 1, &page);
    iso.on_game_tick(1);
    assert!(iso.probe("true").is_ok());
    assert!(iso.drain_interacts().is_empty(), "no posted `here`, no verb");

    // Tick 2: the page posts no stat row at all. That is not a zero.
    post_scene(&iso, 2, &page, &Scene::default());
    iso.on_game_tick(2);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "a page with no `hitpoints` is not a death"
    );

    // Tick 3: a posted stat above zero is not one either.
    post_scene(
        &iso,
        3,
        &page,
        &Scene {
            stats: &healthy,
            ..Scene::default()
        },
    );
    iso.on_game_tick(3);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "a live player is not a death"
    );

    // Tick 4: the posted effective hitpoints read zero: the terminal.
    post_scene(
        &iso,
        4,
        &page,
        &Scene {
            stats: &downed,
            ..Scene::default()
        },
    );
    iso.on_game_tick(4);
    assert!(iso.probe("true").is_ok());
    assert!(iso.drain_interacts().is_empty(), "a death pushes nothing");

    // Tick 5: the token died with it.
    post_scene(&iso, 5, &page, &Scene::default());
    iso.on_game_tick(5);
    assert!(iso.probe("true").is_ok());

    let probed = iso.probe("globalThis.__probe").unwrap();
    let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
    let interacts = iso.drain_interacts();
    iso.join();
    assert!(interacts.is_empty(), "{interacts:?}");
    let steps = value["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 8, "{value:?}");
    for (index, kind) in [
        (1, "callback.enabled"),
        (2, "callback.log"),
        (3, "callback.setStatus"),
        (4, "wait"),
        (5, "wait"),
        (6, "dead"),
    ] {
        assert_eq!(steps[index]["kind"], kind, "{index} {value:?}");
    }
    // The dead step is a bare continue kind: no verb, no reason and no solved
    // mark. The token it carries is the dead one — the session's next call
    // hears `stale` — which is the same post-bump number `aborted` reports.
    assert_eq!(steps[6]["ok"], true, "{value:?}");
    assert_eq!(steps[6]["status"], "continue", "{value:?}");
    assert_ne!(
        steps[6]["token"], value["token"],
        "the death bumps the token it reports: {value:?}"
    );
    for absent in ["action", "name", "x", "z", "level", "message", "id", "error"] {
        assert!(steps[6].get(absent).is_none(), "{absent} {value:?}");
    }
    // And the dead token hears `stale`, never a resumed step.
    assert_eq!(steps[7]["ok"], false, "{value:?}");
    assert_eq!(steps[7]["error"], "stale", "{value:?}");
    assert!(
        !value.to_string().contains("clue solved"),
        "a death is never a solved mark: {value:?}"
    );
}

/// The public `guardian-lost` terminal: the wizard this token Attacked leaves
/// the posted npc page outside the freeze-aware 6000ms grace without ever
/// being seen at zero health — the encounter is lost, the token dies and
/// nothing redigs.
#[test]
fn v2_clue_owned_wizard_gone_outside_the_grace_is_guardian_lost() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__runs = (globalThis.__runs || 0) + 1;
  if (globalThis.__runs === 1) {
    const begin = api.clue.begin();
    globalThis.__token = begin.ok ? begin.value.token : null;
    globalThis.__steps = [begin];
    return;
  }
  globalThis.__steps.push(api.clue.next(
    globalThis.__runs === 3
      ? { token: globalThis.__token, resume: true }
      : { token: globalThis.__token }));
  if (globalThis.__runs === 11) {
    globalThis.__probe = JSON.stringify({
      token: globalThis.__token,
      runs: globalThis.__runs,
      steps: globalThis.__steps,
    });
  }
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso =
        LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data).unwrap();
    let page = [(2723, 1), (952, 1)];
    let names = [(952, "Spade")];
    let attack = vec!["Attack".to_string()];
    let wizard = scene_npc(7, "Zamorak Wizard", 3, 10, 10, &attack);
    let overlay_off = [VarpInput {
        index: 95,
        value: 0,
    }];
    let overlay_on = [VarpInput {
        index: 95,
        value: 1,
    }];
    let far = TileInput {
        x: 3100,
        z: 3300,
        level: 0,
    };
    let arrived = TileInput {
        x: 3058,
        z: 3884,
        level: 0,
    };

    // Ticks 1-4: the begin and the landed report — nothing on the drain.
    for tick in 1..=4 {
        post_scene(&iso, tick, &page, &guarded_scene(far, &names, &[], &[], &[], false, false));
        iso.on_game_tick(tick);
        assert!(iso.probe("true").is_ok());
        assert!(
            iso.drain_interacts().is_empty(),
            "tick {tick} pushes no verb"
        );
    }

    // Tick 5: the walk to the decoded `0_47_60_50_44` tile.
    post_scene(&iso, 5, &page, &guarded_scene(far, &names, &[], &[], &[], false, false));
    iso.on_game_tick(5);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Walk {
            x: 3058,
            z: 3884,
            level: 0,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 0,
        }],
        "the guarded row walks to its own selected pin"
    );

    // Tick 6: arrived with the Spade: the first Dig, which is the spawn.
    post_scene(&iso, 6, &page, &guarded_scene(arrived, &names, &[], &overlay_off, &[], false, false));
    iso.on_game_tick(6);
    assert!(iso.probe("true").is_ok());
    assert_eq!(iso.drain_interacts(), vec![spade_dig()], "the spawn Dig");

    // Tick 7: the wizard posted with the overlay off: the landed click.
    post_scene(
        &iso,
        7,
        &page,
        &guarded_scene(arrived, &names, &[wizard], &overlay_off, &[], false, false),
    );
    iso.on_game_tick(7);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::IfButton { component_id: 5621 }],
        "the overlay off is the click and never an Attack"
    );

    // Tick 8: the overlay reads up: the one Attack, at the posted index.
    post_scene(
        &iso,
        8,
        &page,
        &guarded_scene(arrived, &names, &[wizard], &overlay_on, &[], false, false),
    );
    iso.on_game_tick(8);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Npc {
            name: "Zamorak Wizard".to_string(),
            action: "Attack".to_string(),
            index: Some(7),
        }],
        "the Attack owns the posted index"
    );

    // Tick 9: the owned wizard is still posted and alive: the fight waits, and
    // this call is the last-seen the grace is measured from.
    post_scene(
        &iso,
        9,
        &page,
        &guarded_scene(arrived, &names, &[wizard], &overlay_on, &[], false, false),
    );
    iso.on_game_tick(9);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "the owned wizard is posted and alive"
    );

    // The grace is real time: past it the owned index leaving the page is no
    // longer this token's kill.
    std::thread::sleep(std::time::Duration::from_millis(6_100));
    post_scene(&iso, 10, &page, &guarded_scene(arrived, &names, &[], &overlay_on, &[], false, false));
    iso.on_game_tick(10);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "a lost wizard is no walk and no redig"
    );

    // Tick 11: the token died with the encounter.
    post_scene(&iso, 11, &page, &guarded_scene(arrived, &names, &[], &overlay_on, &[], false, false));
    iso.on_game_tick(11);
    assert!(iso.probe("true").is_ok());

    let probed = iso.probe("globalThis.__probe").unwrap();
    let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
    let interacts = iso.drain_interacts();
    iso.join();
    assert!(interacts.is_empty(), "{interacts:?}");
    let steps = value["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 11, "{value:?}");
    for (index, kind) in [
        (1, "callback.enabled"),
        (2, "callback.log"),
        (3, "callback.setStatus"),
        (4, "walk"),
        (5, "held"),
        (6, "if-button"),
        (7, "npc"),
        (8, "wait"),
        (9, "guardian-lost"),
    ] {
        assert_eq!(steps[index]["kind"], kind, "{index} {value:?}");
    }
    // The loss is a bare continue kind: no verb, no reason, no solved mark,
    // and no `stale` for the step that carried it. The token it carries is the
    // dead one, the same post-bump number `aborted` reports.
    assert_eq!(steps[9]["ok"], true, "{value:?}");
    assert_eq!(steps[9]["status"], "continue", "{value:?}");
    assert_ne!(
        steps[9]["token"], value["token"],
        "the loss bumps the token it reports: {value:?}"
    );
    for absent in ["action", "name", "x", "z", "level", "message", "id", "error"] {
        assert!(steps[9].get(absent).is_none(), "{absent} {value:?}");
    }
    // The encounter ended with the token: the next call is `stale`.
    assert_eq!(steps[10]["ok"], false, "{value:?}");
    assert_eq!(steps[10]["error"], "stale", "{value:?}");
    let text = value.to_string();
    for forbidden in ["clue solved", "abandon", "supplies-needed", "\"done\""] {
        assert!(!text.contains(forbidden), "{forbidden} {value:?}");
    }
}

/// The guarded encounter's npc page is the locked marshal: `clue.next` is
/// handed the posted index, id, name, tile, distance, health pair, `in_combat`
/// flag, actions and target pair and nothing else, so the machine's same-level
/// filter and its Chebyshev read both have the posted page to work from.
#[test]
fn the_guarded_npc_page_marshals_the_locked_field_list() {
    let bindings = include_str!("../src/load/bindings.rs");
    let page = bindings
        .split("function clueNpcPage()")
        .nth(1)
        .expect("clueNpcPage");
    let page = page.split("function clueSelfSlot()").next().expect("page end");
    for field in [
        "index: row.index,",
        "id: cluePageI32(row.id) ? row.id : null,",
        "name: typeof row.name === 'string' ? row.name : null,",
        "x: cluePageI32(row.x) ? row.x : null,",
        "z: cluePageI32(row.z) ? row.z : null,",
        "level: cluePageI32(row.level) ? row.level : null,",
        "distance: cluePageI32(row.distance) ? row.distance : null,",
        "health: cluePageI32(row.health) ? row.health : null,",
        "max_health: cluePageI32(row.max_health) ? row.max_health : null,",
        "in_combat: typeof row.in_combat === 'boolean' ? row.in_combat : null,",
        "actions: Array.isArray(row.actions)",
        "target_kind: cluePageI32(row.target_kind) ? row.target_kind : null,",
        "target_index: cluePageI32(row.target_index) ? row.target_index : null,",
    ] {
        assert!(page.contains(field), "{field} missing from {page}");
    }
    // A row that did not post an index cannot be Attacked and is dropped here:
    // never a snapshot error and never an invented row.
    assert!(page.contains("if (!cluePageI32(row.index)) continue;"), "{page}");
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

/// The public TRAIL-END COLLECT path: `Steady` on the held casket whose Open
/// went out, then the empty page that used to abort. The collect closes the
/// posted reward interface, Takes the casket's own overflow off the posted
/// tile — skipping the shark that is posted first — logs the settled Take,
/// waits the frozen reward window out, and finishes with the landed
/// `none-held` abort. Schema 4, `close-modal` / `obj` unpublished.
#[test]
fn v2_clue_collect_closes_the_reward_then_takes_the_casket_overflow() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__runs = (globalThis.__runs || 0) + 1;
  if (globalThis.__runs === 1) {
    const begin = api.clue.begin();
    globalThis.__token = begin.ok ? begin.value.token : null;
    globalThis.__steps = [begin];
    return;
  }
  globalThis.__steps.push(api.clue.next(
    globalThis.__runs === 3
      ? { token: globalThis.__token, resume: true }
      : { token: globalThis.__token }));
  if (globalThis.__runs === 9) {
    let objOp = null;
    let closeOp = null;
    let heldOp = null;
    try { api.request({ op: 'obj', x: 1, z: 1, level: 0, action: 'Take' }); objOp = 'ok'; }
    catch (e) { objOp = String(e && (e.message || e)); }
    try { api.request({ op: 'close-modal' }); closeOp = 'ok'; }
    catch (e) { closeOp = String(e && (e.message || e)); }
    try { api.request({ op: 'held', name: 'Casket', action: 'Open' }); heldOp = 'ok'; }
    catch (e) { heldOp = String(e && (e.message || e)); }
    globalThis.__probe = JSON.stringify({
      token: globalThis.__token,
      runs: globalThis.__runs,
      steps: globalThis.__steps,
      objOp: objOp,
      closeOp: closeOp,
      heldOp: heldOp,
      groundType: typeof api.snapshot.ground,
      locsType: typeof api.snapshot.locs,
      mainType: typeof api.snapshot.main_modal_id,
      invSizeType: typeof api.snapshot.inv_size,
    });
  }
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data)
        .unwrap();
    // The held sextant028 casket, then the page with nothing membership-held:
    // the Open already went out, so this is the trail-end collect and not an
    // abort. The packed 3554 clue is never played.
    let casket = [(3555, 1)];
    let here = TileInput {
        x: 3222,
        z: 3223,
        level: 1,
    };
    let take = vec!["Take".to_string()];
    let loot = scene_ground(900, "Rune platebody", 3222, 3223, 1, &take);
    let shark = scene_ground(api::clue_pack::SHARK_ID, "Shark", 3222, 3223, 1, &take);
    let ground = vec![loot];
    let both = vec![shark, loot];

    for tick in 1..=5 {
        post_page(&iso, tick, &casket);
        iso.on_game_tick(tick);
        assert!(iso.probe("true").is_ok());
        if tick < 5 {
            assert!(
                iso.drain_interacts().is_empty(),
                "tick {tick} pushes no interact"
            );
        }
    }
    assert_eq!(
        iso.drain_interacts(),
        vec![casket_open()],
        "the landed Open is still the only verb while the casket is held"
    );

    // Tick 6: the casket is gone and the reward interface is posted open.
    post_scene(
        &iso,
        6,
        &[],
        &Scene {
            here: Some(here),
            ground: &ground,
            main_modal_id: 6960,
            ..Scene::default()
        },
    );
    iso.on_game_tick(6);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::CloseModal],
        "the posted main modal is closed, by its own posted id"
    );

    // Tick 7: closed, and the shark posted ahead of the casket overflow is
    // skipped: the Take is the loot row's own tile and name.
    post_scene(
        &iso,
        7,
        &[],
        &Scene {
            here: Some(here),
            ground: &both,
            main_modal_id: -1,
            ..Scene::default()
        },
    );
    iso.on_game_tick(7);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Obj {
            x: 3222,
            z: 3223,
            level: 1,
            name: Some("Rune platebody".to_string()),
            action: "Take".to_string(),
        }],
        "the shark is skipped and the loot keeps its posted identity"
    );

    // Tick 8: only the shark is left, so the Take settled.
    post_scene(
        &iso,
        8,
        &[],
        &Scene {
            here: Some(here),
            ground: &[shark],
            main_modal_id: -1,
            ..Scene::default()
        },
    );
    iso.on_game_tick(8);
    assert!(iso.probe("true").is_ok());
    assert!(iso.drain_interacts().is_empty(), "the settle pushes nothing");

    // Tick 9: an empty page still inside the reward window is a wait, and the
    // unpublished verbs stay unpublished while `held` stays an author op.
    post_scene(
        &iso,
        9,
        &[],
        &Scene {
            here: Some(here),
            main_modal_id: -1,
            ..Scene::default()
        },
    );
    iso.on_game_tick(9);
    let probed = iso.probe("globalThis.__probe").unwrap();
    let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Held {
            name: "Casket".to_string(),
            action: "Open".to_string(),
        }],
        "the drain holds the probe's own `held` request"
    );

    // Past the window the collect is over: the machine's own completion, one
    // step per call — the exact `'clue solved'` status, then the live-token
    // grind handback, then `done`, which kills the token.
    std::thread::sleep(std::time::Duration::from_millis(2_100));
    let empty = Scene {
        here: Some(here),
        main_modal_id: -1,
        ..Scene::default()
    };
    for tick in 10..=12 {
        post_scene(&iso, tick, &[], &empty);
        iso.on_game_tick(tick);
        assert!(iso.probe("true").is_ok());
        assert!(
            iso.drain_interacts().is_empty(),
            "tick {tick}: the completion pushes no verb"
        );
    }
    let steps: serde_json::Value = serde_json::from_str(
        iso.probe("JSON.stringify(globalThis.__steps)")
            .unwrap()
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let interacts = iso.drain_interacts();
    iso.join();
    assert!(interacts.is_empty(), "the finish pushes nothing: {interacts:?}");

    assert_eq!(value["runs"], 9, "{value:?}");
    assert!(value["token"].is_number(), "{value:?}");
    let logged = value["steps"].as_array().expect("steps");
    for (index, kind) in [
        (1, "callback.enabled"),
        (2, "callback.log"),
        (3, "callback.setStatus"),
        (4, "held"),
        (5, "close-modal"),
        (6, "obj"),
        (7, "callback.log"),
        (8, "wait"),
    ] {
        assert_eq!(logged[index]["kind"], kind, "{index} {value:?}");
    }
    // The Open is the casket's and the Take is the loot row's, each with the
    // identity the posted page carried.
    assert_eq!(logged[4]["name"], "Casket", "{value:?}");
    assert_eq!(logged[4]["action"], "Open", "{value:?}");
    assert_eq!(logged[6]["name"], "Rune platebody", "{value:?}");
    assert_eq!(logged[6]["action"], "Take", "{value:?}");
    assert_eq!(logged[6]["x"], 3222, "{value:?}");
    assert_eq!(logged[6]["z"], 3223, "{value:?}");
    assert_eq!(logged[6]["level"], 1, "{value:?}");
    assert_eq!(
        logged[7]["message"], "took 'Rune platebody' from the casket",
        "{value:?}"
    );
    // The collect is the casket's: the landed report never named 3554.
    let report = logged[2]["message"].as_str().unwrap_or("");
    assert!(report.contains("trail_clue_hard_sextant028_casket"), "{value:?}");
    assert!(!report.contains("3554"), "{value:?}");

    // Unpublished verbs stay unpublished, `held` stays an author op, and
    // `ground` / `locs` / `main_modal_id` stay off the public snapshot.
    for key in ["objOp", "closeOp"] {
        assert!(
            value[key].as_str().unwrap_or("").contains("not impl"),
            "{key} {value:?}"
        );
    }
    assert_eq!(value["heldOp"], "ok", "{value:?}");
    for key in ["groundType", "locsType", "mainType"] {
        assert_eq!(value[key], "undefined", "{key} {value:?}");
    }
    assert_eq!(value["invSizeType"], "number", "{value:?}");

    let collected = steps.as_array().expect("steps");
    assert_eq!(collected.len(), 12, "{steps:?}");
    assert_eq!(collected[9]["kind"], "callback.setStatus", "{steps:?}");
    assert_eq!(collected[9]["message"], "clue solved", "{steps:?}");
    assert_eq!(collected[9]["token"], value["token"], "{steps:?}");
    assert_eq!(collected[10]["kind"], "grind-ready", "{steps:?}");
    assert_eq!(collected[10]["ok"], true, "{steps:?}");
    assert_eq!(collected[10]["status"], "continue", "{steps:?}");
    assert_eq!(
        collected[10]["token"], value["token"],
        "the grind handback keeps the live token: {steps:?}"
    );
    assert_eq!(collected[11]["kind"], "done", "{steps:?}");
    assert_eq!(collected[11]["ok"], true, "{steps:?}");
    assert_eq!(collected[11]["status"], "continue", "{steps:?}");
    for absent in ["action", "name", "x", "z", "level", "message", "id", "error"] {
        assert!(collected[11].get(absent).is_none(), "{absent} {steps:?}");
    }
    let text = steps.to_string();
    for forbidden in ["abandon", "supplies-needed", "ownsEquipment"] {
        assert!(!text.contains(forbidden), "{forbidden} {steps:?}");
    }
    // The run-9 snapshot is the collect before the finish: it never carried
    // the solved mark, so the mark is the latch's and nothing else's.
    let early = value.to_string();
    for forbidden in ["clue solved", "\"done\"", "grind-ready"] {
        assert!(!early.contains(forbidden), "{forbidden} {value:?}");
    }
}

/// The collect is skipped whenever identify still holds a step: the landed
/// gate re-arms, so a next scroll keeps reporting and a leftover casket Opens
/// — never a close and never a Take under it, even with both posted.
#[test]
fn v2_clue_collect_re_arms_for_a_held_step_instead_of_collecting() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__runs = (globalThis.__runs || 0) + 1;
  if (globalThis.__runs === 1) {
    const begin = api.clue.begin();
    globalThis.__token = begin.ok ? begin.value.token : null;
    globalThis.__steps = [begin];
    return;
  }
  const resume = globalThis.__runs === 3 || globalThis.__runs === 7;
  globalThis.__steps.push(api.clue.next(
    resume ? { token: globalThis.__token, resume: true } : { token: globalThis.__token }));
  if (globalThis.__runs === 9) {
    globalThis.__probe = JSON.stringify({
      token: globalThis.__token,
      runs: globalThis.__runs,
      steps: globalThis.__steps,
    });
  }
}
"#;
    // The next scroll (a desc-only riddle) and a leftover casket: identify
    // returns a step for both, so collect never runs.
    for (id, final_kind, alias) in [
        (2831, "wait", "trail_clue_medium_riddle001"),
        (3531, "held", "trail_clue_hard_sextant016_casket"),
    ] {
        let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
        let iso =
            LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data)
                .unwrap();
        let casket = [(3555, 1)];
        let next_step = [(id, 1)];
        let here = TileInput {
            x: 3222,
            z: 3223,
            level: 1,
        };
        let take = vec!["Take".to_string()];
        let loot = [scene_ground(900, "Rune platebody", 3222, 3223, 1, &take)];

        for tick in 1..=5 {
            post_page(&iso, tick, &casket);
            iso.on_game_tick(tick);
            assert!(iso.probe("true").is_ok());
        }
        assert_eq!(iso.drain_interacts(), vec![casket_open()], "{id}");

        // The step is still held, with the reward interface and the casket
        // overflow both posted: neither is touched.
        for tick in 6..=9 {
            post_scene(
                &iso,
                tick,
                &next_step,
                &Scene {
                    here: Some(here),
                    ground: &loot,
                    main_modal_id: 6960,
                    ..Scene::default()
                },
            );
            iso.on_game_tick(tick);
            assert!(iso.probe("true").is_ok());
            let interacts = iso.drain_interacts();
            match tick {
                9 if final_kind == "held" => assert_eq!(
                    interacts,
                    vec![casket_open()],
                    "{id}: the leftover casket Opens"
                ),
                _ => assert!(
                    interacts.is_empty(),
                    "{id} tick {tick} pushes no collect verb: {interacts:?}"
                ),
            }
        }

        let probed = iso.probe("globalThis.__probe").unwrap();
        let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
        iso.join();
        assert_eq!(value["runs"], 9, "{id} {value:?}");
        let steps = value["steps"].as_array().expect("steps");
        for (index, kind) in [
            (1, "callback.enabled"),
            (2, "callback.log"),
            (3, "callback.setStatus"),
            (4, "held"),
            (5, "callback.enabled"),
            (6, "callback.log"),
            (7, "callback.setStatus"),
            (8, final_kind),
        ] {
            assert_eq!(steps[index]["kind"], kind, "{id} {index} {value:?}");
        }
        // The re-armed gate reports the row identify returned, never the
        // casket whose Open already went out.
        let message = steps[6]["message"].as_str().unwrap_or("");
        assert!(message.contains(alias), "{id} {value:?}");
        assert!(!message.contains("sextant028_casket"), "{id} {value:?}");
        let text = value.to_string();
        for forbidden in ["clue solved", "\"done\"", "abandon"] {
            assert!(!text.contains(forbidden), "{id} {forbidden} {value:?}");
        }
    }
}

/// The public pack-full path: one Dropped food row frees the slot, the Take
/// follows next call, and a pack that is full with nothing to Drop logs the
/// frozen WARNING and then finishes with the landed `none-held` abort.
#[test]
fn v2_clue_collect_drops_food_or_warns_before_the_take() {
    let take = vec!["Take".to_string()];
    let here = TileInput {
        x: 3222,
        z: 3223,
        level: 1,
    };
    let loot = [scene_ground(900, "Rune platebody", 3222, 3223, 1, &take)];
    let shark = scene_ground(api::clue_pack::SHARK_ID, "Shark", 3222, 3223, 1, &take);

    // Phase one: 28 occupied slots with the hard casket's own shark in the
    // pack, so the Drop is the shark and the Take is next call.
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__runs = (globalThis.__runs || 0) + 1;
  if (globalThis.__runs === 1) {
    const begin = api.clue.begin();
    globalThis.__token = begin.ok ? begin.value.token : null;
    globalThis.__steps = [begin];
    return;
  }
  globalThis.__steps.push(api.clue.next(
    globalThis.__runs === 3
      ? { token: globalThis.__token, resume: true }
      : { token: globalThis.__token }));
  if (globalThis.__runs === 8) {
    globalThis.__probe = JSON.stringify({
      token: globalThis.__token,
      runs: globalThis.__runs,
      steps: globalThis.__steps,
    });
  }
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso =
        LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data).unwrap();
    let casket = [(3555, 1)];
    let mut full: Vec<(i32, i32)> = vec![(api::clue_pack::SHARK_ID, 1)];
    for slot in 0..27 {
        full.push((900_000 + slot, 1));
    }
    let names = [(api::clue_pack::SHARK_ID, "Shark")];
    let full_scene = Scene {
        here: Some(here),
        ground: &loot,
        main_modal_id: -1,
        names: &names,
        ..Scene::default()
    };
    let take_scene = Scene {
        here: Some(here),
        ground: &[shark, loot[0]],
        main_modal_id: -1,
        names: &names,
        ..Scene::default()
    };
    let shark_scene = Scene {
        here: Some(here),
        ground: &[shark],
        main_modal_id: -1,
        names: &names,
        ..Scene::default()
    };

    for tick in 1..=5 {
        post_page(&iso, tick, &casket);
        iso.on_game_tick(tick);
        assert!(iso.probe("true").is_ok());
    }
    assert_eq!(iso.drain_interacts(), vec![casket_open()]);

    // Tick 6: the pack is full and the loot is on the tile — the Drop.
    post_scene(&iso, 6, &full, &full_scene);
    iso.on_game_tick(6);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Held {
            name: "Shark".to_string(),
            action: "Drop".to_string(),
        }],
        "the hard casket drops the frozen SHARK_ID row"
    );

    // Tick 7: one slot freed. The row the Drop put on the floor is not the
    // row that is taken.
    let freed: Vec<(i32, i32)> = full[1..].to_vec();
    post_scene(&iso, 7, &freed, &take_scene);
    iso.on_game_tick(7);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Obj {
            x: 3222,
            z: 3223,
            level: 1,
            name: Some("Rune platebody".to_string()),
            action: "Take".to_string(),
        }],
        "the Take is the loot, not the dropped food"
    );

    // Tick 8: the loot left the ground: the settled Take line.
    post_scene(&iso, 8, &freed, &shark_scene);
    iso.on_game_tick(8);
    assert!(iso.probe("true").is_ok());
    let probed = iso.probe("globalThis.__probe").unwrap();
    let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
    let interacts = iso.drain_interacts();
    iso.join();
    assert!(interacts.is_empty(), "{interacts:?}");
    let steps = value["steps"].as_array().expect("steps");
    for (index, kind) in [
        (1, "callback.enabled"),
        (2, "callback.log"),
        (3, "callback.setStatus"),
        (4, "held"),
        (5, "held"),
        (6, "obj"),
        (7, "callback.log"),
    ] {
        assert_eq!(steps[index]["kind"], kind, "{index} {value:?}");
    }
    assert_eq!(steps[5]["action"], "Drop", "{value:?}");
    assert_eq!(steps[5]["name"], "Shark", "{value:?}");
    assert_eq!(
        steps[7]["message"], "took 'Rune platebody' from the casket",
        "{value:?}"
    );

    // Phase two: the pack is full with nothing droppable, so the frozen
    // WARNING is logged and the next call is the landed `none-held` abort.
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__runs = (globalThis.__runs || 0) + 1;
  if (globalThis.__runs === 1) {
    const begin = api.clue.begin();
    globalThis.__token = begin.ok ? begin.value.token : null;
    globalThis.__steps = [begin];
    return;
  }
  globalThis.__steps.push(api.clue.next(
    globalThis.__runs === 3
      ? { token: globalThis.__token, resume: true }
      : { token: globalThis.__token }));
  if (globalThis.__runs === 7) {
    globalThis.__probe = JSON.stringify({
      token: globalThis.__token,
      runs: globalThis.__runs,
      steps: globalThis.__steps,
    });
  }
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso =
        LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data).unwrap();
    let no_food: Vec<(i32, i32)> = (0..28).map(|slot| (900_000 + slot, 1)).collect();
    for tick in 1..=5 {
        post_page(&iso, tick, &casket);
        iso.on_game_tick(tick);
        assert!(iso.probe("true").is_ok());
    }
    assert_eq!(iso.drain_interacts(), vec![casket_open()]);

    // Tick 6: full pack, loot waiting, nothing to Drop.
    let scene = Scene {
        here: Some(here),
        ground: &loot,
        main_modal_id: -1,
        ..Scene::default()
    };
    post_scene(&iso, 6, &no_food, &scene);
    iso.on_game_tick(6);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "the WARNING is a log line, not a verb"
    );

    // Ticks 7-9: the WARNING ended the collect, so the collect's own
    // completion follows — the exact `'clue solved'` status, the live-token
    // grind handback, then `done`, which kills the token.
    for tick in 7..=9 {
        post_scene(&iso, tick, &no_food, &scene);
        iso.on_game_tick(tick);
        assert!(iso.probe("true").is_ok());
        assert!(
            iso.drain_interacts().is_empty(),
            "tick {tick}: the completion is not a verb"
        );
    }
    let probed = iso.probe("globalThis.__probe").unwrap();
    let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
    let collected: serde_json::Value = serde_json::from_str(
        iso.probe("JSON.stringify(globalThis.__steps)")
            .unwrap()
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let interacts = iso.drain_interacts();
    iso.join();
    assert!(interacts.is_empty(), "{interacts:?}");
    let steps = value["steps"].as_array().expect("steps");
    assert_eq!(steps[5]["kind"], "callback.log", "{value:?}");
    assert_eq!(
        steps[5]["message"],
        "WARNING: 'Rune platebody' is left on the ground, the pack is full with no Shark to drop",
        "{value:?}"
    );
    assert_eq!(steps[5]["token"], value["token"], "{value:?}");
    let collected = collected.as_array().expect("steps");
    assert_eq!(collected.len(), 9, "{collected:?}");
    assert_eq!(collected[6]["kind"], "callback.setStatus", "{collected:?}");
    assert_eq!(collected[6]["message"], "clue solved", "{collected:?}");
    assert_eq!(collected[6]["token"], value["token"], "{collected:?}");
    assert_eq!(collected[7]["kind"], "grind-ready", "{collected:?}");
    assert_eq!(collected[7]["ok"], true, "{collected:?}");
    assert_eq!(collected[7]["status"], "continue", "{collected:?}");
    assert_eq!(
        collected[7]["token"], value["token"],
        "the grind handback keeps the live token: {collected:?}"
    );
    assert_eq!(collected[8]["kind"], "done", "{collected:?}");
    assert_eq!(collected[8]["ok"], true, "{collected:?}");
    assert_eq!(collected[8]["status"], "continue", "{collected:?}");
    // The run-7 snapshot ends on the collect's own finish, so the solved mark
    // is nowhere before it: the Open and the WARNING never carry it.
    for index in [4, 5] {
        assert!(
            !steps[index].to_string().contains("clue solved"),
            "{index} {value:?}"
        );
    }
    let early = value.to_string();
    for forbidden in ["abandon", "ownsEquipment"] {
        assert!(!early.contains(forbidden), "{forbidden} {value:?}");
    }
}

/// Freeze and yield beat the public collect arm the way they beat the landed
/// verbs: a frozen call emits no close, no Take and no Drop, and the collect
/// picks the page up unchanged afterwards.
#[test]
fn v2_clue_freeze_and_yield_beat_the_collect() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__runs = (globalThis.__runs || 0) + 1;
  if (globalThis.__runs === 1) {
    const begin = api.clue.begin();
    globalThis.__token = begin.ok ? begin.value.token : null;
    globalThis.__steps = [begin];
    return;
  }
  globalThis.__steps.push(api.clue.next(
    globalThis.__runs === 3
      ? { token: globalThis.__token, resume: true }
      : { token: globalThis.__token }));
  if (globalThis.__runs === 7) {
    globalThis.__probe = JSON.stringify({
      token: globalThis.__token,
      runs: globalThis.__runs,
      steps: globalThis.__steps,
    });
  }
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso =
        LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data).unwrap();
    let casket = [(3555, 1)];
    let here = TileInput {
        x: 3222,
        z: 3223,
        level: 1,
    };
    let take = vec!["Take".to_string()];
    let loot = [scene_ground(900, "Rune platebody", 3222, 3223, 1, &take)];
    let reward = Scene {
        here: Some(here),
        ground: &loot,
        main_modal_id: 6960,
        ..Scene::default()
    };

    for tick in 1..=5 {
        post_page(&iso, tick, &casket);
        iso.on_game_tick(tick);
        assert!(iso.probe("true").is_ok());
    }
    assert_eq!(iso.drain_interacts(), vec![casket_open()]);

    // Tick 6: the posted `hold` freezes the machine and is a paint-only tick,
    // so the script never runs and no close is sent for the posted reward.
    post_scene(
        &iso,
        6,
        &[],
        &Scene {
            hold: true,
            ..reward
        },
    );
    iso.on_game_tick(6);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "a frozen call emits no close and no Take"
    );
    assert_eq!(
        iso.probe("String(globalThis.__runs)").unwrap().as_str(),
        Some("5"),
        "the posted hold skipped the tick's script, and it skipped no verb"
    );

    // Tick 7: the posted `ours` interrupt with an unfrozen clock: yield, and
    // neither the close nor the Take rides along with it.
    post_scene(
        &iso,
        7,
        &[],
        &Scene {
            ours: true,
            ..reward
        },
    );
    iso.on_game_tick(7);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "yield emits no close and no Take"
    );

    // Tick 8: thawed and unheld, the collect picks the same page up.
    post_scene(&iso, 8, &[], &reward);
    iso.on_game_tick(8);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::CloseModal],
        "the posted reward is still there after the interrupt"
    );

    let probed = iso.probe("globalThis.__probe").unwrap();
    let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
    iso.join();
    assert_eq!(value["runs"], 7, "{value:?}");
    let steps = value["steps"].as_array().expect("steps");
    for (index, kind) in [
        (1, "callback.enabled"),
        (2, "callback.log"),
        (3, "callback.setStatus"),
        (4, "held"),
        (5, "yield"),
        (6, "close-modal"),
    ] {
        assert_eq!(steps[index]["kind"], kind, "{index} {value:?}");
    }
    assert_eq!(steps[6]["token"], value["token"], "{value:?}");
    let text = value.to_string();
    for forbidden in ["clue solved", "\"done\"", "ownsEquipment"] {
        assert!(!text.contains(forbidden), "{forbidden} {value:?}");
    }
}


/// One held puzzle step's posted pack page: the desc-only riddle and its own
/// selected box.
fn puzzle_page() -> Vec<(i32, i32)> {
    vec![(PUZZLE_RIDDLE, 1), (PUZZLE_BOX, 1)]
}

/// One held box Open as the drain sees it: the selected item display name the
/// host resolves, and the frozen action. No row id and no tile.
fn puzzle_open() -> InteractReq {
    InteractReq::Held {
        name: "Puzzle box".to_string(),
        action: "Open".to_string(),
    }
}

/// One posted board page over a board's rows, at the board session the click
/// carries.
fn board_scene<'a>(rows: &'a [script::isolate_fb::ItemRowInput<'a>]) -> Scene<'a> {
    Scene {
        puzzle: Some(PostedPuzzle {
            component_id: BOARD_COMPONENT,
            size: 25,
            items: rows,
            generation: 7,
        }),
        ..Scene::default()
    }
}

/// The closed board SNAP posts beside a box that is not open: the present
/// empty observation, which is not the same page as no board table at all.
fn closed_board_scene() -> Scene<'static> {
    Scene {
        puzzle: Some(PostedPuzzle {
            component_id: -1,
            size: 0,
            items: &[],
            generation: 8,
        }),
        ..Scene::default()
    }
}

/// The public `api.clue.next` path over a posted puzzle board: the held box's
/// own `held` Open reaches the drain as `InteractReq::Held`, one planned click
/// reaches it as `InteractReq::PuzzleMove` — never as a loc and never as an
/// inventory button — and the solved board reaches it as
/// `InteractReq::CloseModal`. The step kinds are the machine's own, so a click
/// that never made it onto the whitelist would surface here as a `stale` error
/// instead of a verb.
#[test]
fn v2_clue_puzzle_box_opens_plans_and_closes_over_the_posted_board() {
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
      // The report is posted: these two are the box's own Open, and they
      // repeat while the board stays the closed one.
      api.clue.next({ token: globalThis.__token }),
      api.clue.next({ token: globalThis.__token }),
    ];
    return;
  }
  globalThis.__steps.push(api.clue.next({ token: globalThis.__token }));
  if (globalThis.__steps.length === 10) {
    globalThis.__probe = JSON.stringify({
      token: globalThis.__token,
      runs: globalThis.__runs,
      steps: globalThis.__steps,
    });
  }
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso =
        LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data).unwrap();
    let page = puzzle_page();

    // Tick 1: the report, then the box's own Open — twice, because the board is
    // still the closed one SNAP posts beside an unopened box.
    post_scene(&iso, 1, &page, &Scene::default());
    iso.on_game_tick(1);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![puzzle_open(), puzzle_open()],
        "each held call opens the box while the board stays closed"
    );

    // Tick 2: the readable board is one slide from solved, so the plan is the
    // single click on the piece beside the gap.
    let rows = board_rows(&one_move_board(), BOARD_COMPONENT);
    post_scene(&iso, 2, &page, &board_scene(&rows));
    iso.on_game_tick(2);
    assert!(iso.probe("true").is_ok());
    let moved = iso.drain_interacts();
    assert_eq!(
        moved,
        vec![InteractReq::PuzzleMove {
            id: PIECE_B + 23,
            slot: 24,
            component: BOARD_COMPONENT,
            generation: 7,
        }],
        "one planned click, at the posted row's own identity: {moved:?}"
    );

    // Tick 3: the posted `hold` freezes this machine's clock and is a
    // paint-only tick — the script never runs — so no click and no close is
    // re-sent.
    post_scene(
        &iso,
        3,
        &page,
        &Scene {
            hold: true,
            ..board_scene(&rows)
        },
    );
    iso.on_game_tick(3);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "a frozen call emits no click and no close"
    );

    // Tick 4: the click landed — the live board is exactly the board that move
    // was expected to produce — and that board is the solved one.
    let solved = board_rows(&solved_board(), BOARD_COMPONENT);
    post_scene(&iso, 4, &page, &board_scene(&solved));
    iso.on_game_tick(4);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::CloseModal],
        "the solved board is closed"
    );

    // Tick 5: the board is gone with the box still held. The step idles.
    post_scene(&iso, 5, &page, &Scene::default());
    iso.on_game_tick(5);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "the closed step opens and closes nothing again"
    );

    // Tick 6: a solved board posted again is not a second close either: the
    // solved-or-attempted latch belongs to the step, not to the board page.
    post_scene(&iso, 6, &page, &board_scene(&solved));
    iso.on_game_tick(6);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "the latch holds while the step stays held"
    );

    let probed = iso.probe("globalThis.__probe").unwrap();
    let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
    iso.join();

    assert!(value["token"].is_number(), "{value:?}");
    // Five script runs: the `hold` tick is paint-only and ran no step.
    assert_eq!(value["runs"], 5, "{value:?}");
    let steps = value["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 10, "{value:?}");
    for (index, kind) in [
        (1, "callback.enabled"),
        (2, "callback.log"),
        (3, "callback.setStatus"),
        (4, "held"),
        (5, "held"),
        (6, "puzzle-move"),
        (7, "close-modal"),
        (8, "wait"),
        (9, "wait"),
    ] {
        assert_eq!(steps[index]["kind"], kind, "{index} {value:?}");
        assert_eq!(steps[index]["ok"], true, "{index} {value:?}");
        assert_eq!(steps[index]["status"], "continue", "{index} {value:?}");
        assert_eq!(steps[index]["token"], value["token"], "{index} {value:?}");
        assert!(steps[index].get("error").is_none(), "{index} {value:?}");
    }
    for index in [4, 5] {
        let step = &steps[index];
        assert_eq!(step["name"], "Puzzle box", "{index} {value:?}");
        assert_eq!(step["action"], "Open", "{index} {value:?}");
        for absent in ["id", "x", "z", "level", "slot", "component"] {
            assert!(step.get(absent).is_none(), "{index} {absent} {step}");
        }
    }
    let click = &steps[6];
    assert_eq!(click["id"], PIECE_B + 23, "{value:?}");
    assert_eq!(click["slot"], 24, "{value:?}");
    assert_eq!(click["component"], BOARD_COMPONENT, "{value:?}");
    assert_eq!(click["generation"], 7, "{value:?}");
    let text = value.to_string();
    for forbidden in ["clue solved", "abandon", "ownsEquipment", "no-puzzle"] {
        assert!(!text.contains(forbidden), "{forbidden} {value:?}");
    }
}

/// The unreadable exit of a live board on the public path: once the box's Open
/// has landed and a click is outstanding, a page still posting a live size-25
/// board whose pieces the selected map cannot place reaches the drain as one
/// `InteractReq::CloseModal` — not a bare latch with the modal open — and the
/// closed page the landed close leaves is not a second close.
#[test]
fn v2_clue_puzzle_unreadable_mid_solve_closes_the_modal_once() {
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
      // The report is posted: the box's own Open, repeating while the board
      // stays the closed one.
      api.clue.next({ token: globalThis.__token }),
      api.clue.next({ token: globalThis.__token }),
    ];
    return;
  }
  globalThis.__steps.push(api.clue.next({ token: globalThis.__token }));
  if (globalThis.__steps.length === 10) {
    globalThis.__probe = JSON.stringify({
      token: globalThis.__token,
      runs: globalThis.__runs,
      steps: globalThis.__steps,
    });
  }
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso =
        LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data).unwrap();
    let page = puzzle_page();

    // Tick 1: the box's own Open, twice, over the closed board SNAP posts
    // beside a box that is not open yet.
    post_scene(&iso, 1, &page, &closed_board_scene());
    iso.on_game_tick(1);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![puzzle_open(), puzzle_open()],
        "each held call opens the box while the board stays closed"
    );

    // Tick 2: the board is open and one slide from solved, so the plan is the
    // single click beside the gap.
    let mut moving = board_rows(&one_move_board(), BOARD_COMPONENT);
    post_scene(&iso, 2, &page, &board_scene(&moving));
    iso.on_game_tick(2);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::PuzzleMove {
            id: PIECE_B + 23,
            slot: 24,
            component: BOARD_COMPONENT,
            generation: 7,
        }],
        "one planned click while the board is readable"
    );

    // Tick 3: the page still posts a live size-25 board, but one piece the
    // selected map cannot place — an open modal nobody can read. The attempt
    // exits through the one close rather than idling with the modal up.
    moving[0].id = PIECE_B + 99;
    post_scene(&iso, 3, &page, &board_scene(&moving));
    iso.on_game_tick(3);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::CloseModal],
        "the unreadable board of an opened box is closed"
    );

    // Tick 4: the board is gone — the closed page the landed close leaves —
    // and the step waits the close window out without a second close.
    post_scene(&iso, 4, &page, &closed_board_scene());
    iso.on_game_tick(4);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "the landed close is never sent twice"
    );

    // Tick 5: a readable solved board posted again is not a second attempt
    // either: the latch belongs to the step, not to the board page.
    let solved = board_rows(&solved_board(), BOARD_COMPONENT);
    post_scene(&iso, 5, &page, &board_scene(&solved));
    iso.on_game_tick(5);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "the latch holds while the step stays held"
    );

    let probed = iso.probe("globalThis.__probe").unwrap();
    let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
    iso.join();

    assert!(value["token"].is_number(), "{value:?}");
    assert_eq!(value["runs"], 5, "{value:?}");
    let steps = value["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 10, "{value:?}");
    for (index, kind) in [
        (1, "callback.enabled"),
        (2, "callback.log"),
        (3, "callback.setStatus"),
        (4, "held"),
        (5, "held"),
        (6, "puzzle-move"),
        (7, "close-modal"),
        (8, "wait"),
        (9, "wait"),
    ] {
        assert_eq!(steps[index]["kind"], kind, "{index} {value:?}");
        assert_eq!(steps[index]["ok"], true, "{index} {value:?}");
        assert_eq!(steps[index]["status"], "continue", "{index} {value:?}");
        assert_eq!(steps[index]["token"], value["token"], "{index} {value:?}");
        assert!(steps[index].get("error").is_none(), "{index} {value:?}");
    }
}

/// The row's own box is what arms the arm: the same desc-only riddle with its
/// box absent from the pack keeps the idle it had, even with a solved board
/// posted.
#[test]
fn v2_clue_puzzle_row_without_its_held_box_stays_idle() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const begin = api.clue.begin();
  const token = begin.ok ? begin.value.token : null;
  globalThis.__steps = [
    begin,
    api.clue.next({ token: token }),
    api.clue.next({ token: token, resume: true }),
    api.clue.next({ token: token }),
    // The report is posted: this is the call every other arm acts on.
    api.clue.next({ token: token }),
  ];
  globalThis.__probe = JSON.stringify({ steps: globalThis.__steps });
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso =
        LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data).unwrap();
    let solved = board_rows(&solved_board(), BOARD_COMPONENT);
    post_scene(&iso, 1, &[(PUZZLE_RIDDLE, 1)], &board_scene(&solved));
    iso.on_game_tick(1);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "an unheld box is not a puzzle step"
    );

    let probed = iso.probe("globalThis.__probe").unwrap();
    let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
    iso.join();
    let steps = value["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 5, "{value:?}");
    assert_eq!(steps[4]["kind"], "wait", "{value:?}");
    assert_eq!(steps[4]["ok"], true, "{value:?}");
}

/// Another row's box in the pack is never this row's: the desc-only riddle
/// with no `_puzzlebox` sibling of its own keeps the idle, and a solved board
/// posted beside it is not closed by this arm.
#[test]
fn v2_clue_riddle_without_a_box_keeps_the_idle() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const begin = api.clue.begin();
  const token = begin.ok ? begin.value.token : null;
  globalThis.__steps = [
    begin,
    api.clue.next({ token: token }),
    api.clue.next({ token: token, resume: true }),
    api.clue.next({ token: token }),
    api.clue.next({ token: token }),
  ];
  globalThis.__probe = JSON.stringify({ steps: globalThis.__steps });
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso =
        LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data).unwrap();
    let solved = board_rows(&solved_board(), BOARD_COMPONENT);
    post_scene(
        &iso,
        1,
        &[(PUZZLE_RIDDLE_NO_BOX, 1), (PUZZLE_BOX, 1)],
        &board_scene(&solved),
    );
    iso.on_game_tick(1);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "a row whose own box it is not opens and closes nothing"
    );

    let probed = iso.probe("globalThis.__probe").unwrap();
    let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
    iso.join();
    let steps = value["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 5, "{value:?}");
    assert_eq!(steps[4]["kind"], "wait", "{value:?}");
}

/// A search row keeps the search arm with a box sitting in the pack: the join
/// is the row's own alias, so no box can steal it.
#[test]
fn v2_clue_search_row_with_a_box_in_the_pack_still_walks() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const begin = api.clue.begin();
  const token = begin.ok ? begin.value.token : null;
  globalThis.__steps = [
    begin,
    api.clue.next({ token: token }),
    api.clue.next({ token: token, resume: true }),
    api.clue.next({ token: token }),
    // Far from the decoded tile: the search arm's own walk.
    api.clue.next({ token: token }),
  ];
  globalThis.__probe = JSON.stringify({ steps: globalThis.__steps });
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso =
        LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data).unwrap();
    let solved = board_rows(&solved_board(), BOARD_COMPONENT);
    post_scene(
        &iso,
        1,
        &[(PUZZLE_SEARCH, 1), (PUZZLE_BOX, 1)],
        &Scene {
            here: Some(TileInput {
                x: 3100,
                z: 3300,
                level: 1,
            }),
            ..board_scene(&solved)
        },
    );
    iso.on_game_tick(1);
    assert!(iso.probe("true").is_ok());
    let walked = iso.drain_interacts();
    assert_eq!(
        walked,
        vec![InteractReq::Walk {
            x: 3209,
            z: 3218,
            level: 1,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 0,
        }],
        "the search row still walks: {walked:?}"
    );

    let probed = iso.probe("globalThis.__probe").unwrap();
    let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
    iso.join();
    let steps = value["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 5, "{value:?}");
    assert_eq!(steps[4]["kind"], "walk", "{value:?}");
}

/// `trail_clue_easy_simple005`: the talk membership whose jm2 spawn is unique —
/// `hans` at `(3207, 3233, plane 0)`.
const TALK_ID: i32 = 2681;
const TALK_X: i32 = 3207;
const TALK_Z: i32 = 3233;
/// `trail_clue_easy_simple008`: the identity-only talk step — `Tanner`, packed
/// id 804 — whose jm2 spawn is not unique, so the family publishes no tile.
const TALK_IDENTITY_ID: i32 = 2684;
const TANNER_ID: i32 = 804;
/// `trail_clue_medium_anagram001`: the challenge parent `Hazelmere`, whose
/// `2842` scroll answers `"6859"`.
const TALK_CHALLENGE_ID: i32 = 2841;
const CHALLENGE_ID: i32 = 2842;
const CHALLENGE_ANSWER: i32 = 6859;

/// One posted npc row as SNAP posts it and the talk arm reads it: the posted
/// index the Talk-to carries, the packed id and posted display name the
/// identity join compares, the posted tile and distance the arrival is measured
/// by, and the posted action list the talk action is read from.
fn talk_scene_npc<'a>(
    index: i32,
    id: i32,
    name: &'a str,
    tile: TileInput,
    distance: i32,
    actions: &'a [String],
) -> script::isolate_fb::SceneEntityInput<'a> {
    SceneEntityInput {
        index,
        id,
        name: Some(name),
        x: tile.x,
        z: tile.z,
        level: tile.level,
        distance,
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
        nx: tile.x,
        nz: tile.z,
    }
}

/// The public `api.clue.next` path over a posted talk scene: the machine's own
/// `walk` and `npc` kinds reach the interact drain as `InteractReq::Walk` /
/// `InteractReq::Npc`, and the arm never Talks-to anything but a posted npc of
/// the step's own identity standing on the published tile.
#[test]
fn v2_clue_unique_spawn_talk_step_walks_then_talks_over_the_posted_scene() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__runs = (globalThis.__runs || 0) + 1;
  const runs = globalThis.__runs;
  if (runs === 1) {
    const begin = api.clue.begin();
    globalThis.__token = begin.ok ? begin.value.token : null;
    globalThis.__steps = [
      begin,
      api.clue.next({ token: globalThis.__token }),
      api.clue.next({ token: globalThis.__token, resume: true }),
      api.clue.next({ token: globalThis.__token }),
      // No posted `here`: the talk arm waits rather than walking blind.
      api.clue.next({ token: globalThis.__token }),
    ];
    return;
  }
  if (runs === 10) {
    // The empty pack page: the begin is the landed C3 refusal.
    globalThis.__steps.push(api.clue.begin());
    globalThis.__probe = JSON.stringify({
      token: globalThis.__token,
      runs: runs,
      steps: globalThis.__steps,
      challengeAnswer: typeof (globalThis.__rs_api.clue.challengeAnswer),
    });
    return;
  }
  globalThis.__steps.push(api.clue.next({ token: globalThis.__token }));
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso =
        LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data).unwrap();
    let page = [(TALK_ID, 1)];
    let talk_to = vec!["Talk-to".to_string()];

    // Tick 1: the held talk step with no posted scene at all.
    post_page(&iso, 1, &page);
    iso.on_game_tick(1);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "a missing `here` is a wait, not a walk"
    );

    // Tick 2: posted far from the published tile: the walk is that tile.
    post_scene(
        &iso,
        2,
        &page,
        &Scene {
            here: Some(TileInput {
                x: 3100,
                z: 3233,
                level: 0,
            }),
            ..Scene::default()
        },
    );
    iso.on_game_tick(2);
    assert!(iso.probe("true").is_ok());
    let walked = iso.drain_interacts();
    assert_eq!(
        walked,
        vec![InteractReq::Walk {
            x: TALK_X,
            z: TALK_Z,
            level: 0,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 0,
        }],
        "the walk is the published x/z/plane: {walked:?}"
    );

    // Tick 3: arrived, with the step's own npc posted on the tile.
    let on_tile = [talk_scene_npc(
        3,
        0,
        "Hans",
        TileInput {
            x: TALK_X,
            z: TALK_Z,
            level: 0,
        },
        1,
        &talk_to,
    )];
    post_scene(
        &iso,
        3,
        &page,
        &Scene {
            here: Some(TileInput {
                x: TALK_X,
                z: TALK_Z,
                level: 0,
            }),
            npcs: &on_tile,
            ..Scene::default()
        },
    );
    iso.on_game_tick(3);
    assert!(iso.probe("true").is_ok());
    let talked = iso.drain_interacts();
    assert_eq!(
        talked,
        vec![InteractReq::Npc {
            name: "Hans".to_string(),
            action: "Talk-to".to_string(),
            index: Some(3),
        }],
        "the Talk-to is the posted identity with the posted index: {talked:?}"
    );

    // Tick 4: the same npc has wandered four tiles off the published tile. The
    // arm keeps the tile and waits: no second target, no Clear and no verb.
    let wandered = [talk_scene_npc(
        4,
        0,
        "Hans",
        TileInput {
            x: TALK_X + 4,
            z: TALK_Z,
            level: 0,
        },
        4,
        &talk_to,
    )];
    post_scene(
        &iso,
        4,
        &page,
        &Scene {
            here: Some(TileInput {
                x: TALK_X,
                z: TALK_Z,
                level: 0,
            }),
            npcs: &wandered,
            ..Scene::default()
        },
    );
    iso.on_game_tick(4);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "a wanderer is a wait, never a chase"
    );

    // Tick 5: the npc is back on the tile but this token owns an open chat:
    // no Talk-to rides an open chat.
    post_scene(
        &iso,
        5,
        &page,
        &Scene {
            here: Some(TileInput {
                x: TALK_X,
                z: TALK_Z,
                level: 0,
            }),
            npcs: &on_tile,
            chat_modal_id: 968,
            ..Scene::default()
        },
    );
    iso.on_game_tick(5);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "an open chat blocks the Talk-to"
    );

    // Tick 6: the posted closed chat — `-1` — is not an open one.
    post_scene(
        &iso,
        6,
        &page,
        &Scene {
            here: Some(TileInput {
                x: TALK_X,
                z: TALK_Z,
                level: 0,
            }),
            npcs: &on_tile,
            chat_modal_id: -1,
            ..Scene::default()
        },
    );
    iso.on_game_tick(6);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Npc {
            name: "Hans".to_string(),
            action: "Talk-to".to_string(),
            index: Some(3),
        }],
        "a closed chat leaves the Talk-to free"
    );

    // Tick 7: the posted `hold || ours` interrupt: yield, and no verb with it.
    post_scene(
        &iso,
        7,
        &page,
        &Scene {
            here: Some(TileInput {
                x: TALK_X,
                z: TALK_Z,
                level: 0,
            }),
            npcs: &on_tile,
            ours: true,
            ..Scene::default()
        },
    );
    iso.on_game_tick(7);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "yield enqueues no Talk-to"
    );

    // Tick 8: a posted hitpoints at zero kills the token, whatever the step.
    let dead = [StatInput {
        index: 3,
        name: "hitpoints",
        xp: 0,
        base: 40,
        effective: 0,
    }];
    post_scene(
        &iso,
        8,
        &page,
        &Scene {
            here: Some(TileInput {
                x: TALK_X,
                z: TALK_Z,
                level: 0,
            }),
            npcs: &on_tile,
            stats: &dead,
            ..Scene::default()
        },
    );
    iso.on_game_tick(8);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "a death pushes no interact"
    );

    // Tick 9: the dead token is `stale`, and the scene it left behind is still
    // a page: nothing was looted and nothing was solved.
    post_scene(
        &iso,
        9,
        &page,
        &Scene {
            here: Some(TileInput {
                x: TALK_X,
                z: TALK_Z,
                level: 0,
            }),
            npcs: &on_tile,
            ..Scene::default()
        },
    );
    iso.on_game_tick(9);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "a dead token has no verbs"
    );

    // Tick 10: the empty pack page refuses the begin with the landed C3 token,
    // and no token is handed out for it.
    post_page(&iso, 10, &[]);
    iso.on_game_tick(10);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "a refused begin has no verbs"
    );

    let probed = iso.probe("globalThis.__probe").unwrap();
    let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
    iso.join();
    let steps = value["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 14, "{value:?}");
    for (index, kind) in [
        (1, "callback.enabled"),
        (2, "callback.log"),
        (3, "callback.setStatus"),
        (4, "wait"),
        (5, "walk"),
        (6, "npc"),
        (7, "wait"),
        (8, "wait"),
        (9, "npc"),
        (10, "yield"),
        (11, "dead"),
        (12, "stale"),
    ] {
        match kind {
            "dead" => {
                assert_eq!(steps[index]["ok"], true, "{index} {value:?}");
                assert_eq!(steps[index]["kind"], "dead", "{index} {value:?}");
            }
            "stale" => {
                assert_eq!(steps[index]["ok"], false, "{index} {value:?}");
                assert_eq!(steps[index]["error"], "stale", "{index} {value:?}");
            }
            _ => assert_eq!(steps[index]["kind"], kind, "{index} {value:?}"),
        }
    }
    // The empty page is still `none-held`, taken through the public helper.
    assert_eq!(steps[13]["ok"], false, "{value:?}");
    assert_eq!(steps[13]["error"], "none-held", "{value:?}");
    // The messages stay the landed identity, and never a completion.
    let logged = steps[2]["message"].as_str().unwrap_or("");
    assert!(logged.contains("trail_clue_easy_simple005"), "{value:?}");
    assert!(logged.contains(&TALK_ID.to_string()), "{value:?}");
    let text = value.to_string();
    for forbidden in ["clue solved", "abandon", "supplies-needed", "ownsEquipment"] {
        assert!(!text.contains(forbidden), "{value:?}");
    }
    // The fence holds on the public path: no `challengeAnswer` is published.
    assert_eq!(value["challengeAnswer"], "undefined", "{value:?}");
}

/// The public `api.clue.next` path over the identity-only talk step: a page
/// that posts no npc of the step's own identity is a `wait` with no verb at
/// all, and the posted match is walked to and then Talk-to'd. The step
/// publishes no tile, so the walk target is the posted row's own.
#[test]
fn v2_clue_identity_only_talk_step_picks_the_posted_match_over_the_scene() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__runs = (globalThis.__runs || 0) + 1;
  const runs = globalThis.__runs;
  if (runs === 1) {
    const begin = api.clue.begin();
    globalThis.__token = begin.ok ? begin.value.token : null;
    globalThis.__steps = [
      begin,
      api.clue.next({ token: globalThis.__token }),
      api.clue.next({ token: globalThis.__token, resume: true }),
      api.clue.next({ token: globalThis.__token }),
      // No posted `here`: the identity-only arm has no base to measure by.
      api.clue.next({ token: globalThis.__token }),
    ];
    return;
  }
  if (runs === 6) {
    globalThis.__probe = JSON.stringify({
      token: globalThis.__token,
      runs: runs,
      steps: globalThis.__steps,
    });
    return;
  }
  globalThis.__steps.push(api.clue.next({ token: globalThis.__token }));
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso =
        LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data).unwrap();
    let page = [(TALK_IDENTITY_ID, 1)];
    let talk_to = vec!["Talk-to".to_string()];

    // Tick 1: the held identity-only step with no posted scene at all.
    post_page(&iso, 1, &page);
    iso.on_game_tick(1);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "a missing `here` is a wait, not a walk"
    );

    // Tick 2: a posted page with no npc of this step's identity — and one that
    // belongs to another talk step entirely. Nothing is picked: no walk, no
    // Talk-to and no nearest anything.
    let other = [talk_scene_npc(4, 0, "Hans", TileInput { x: 3200, z: 3205, level: 0 }, 1, &talk_to)];
    post_scene(
        &iso,
        2,
        &page,
        &Scene {
            here: Some(TileInput {
                x: 3200,
                z: 3200,
                level: 0,
            }),
            npcs: &other,
            ..Scene::default()
        },
    );
    iso.on_game_tick(2);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "no posted match is a wait, not an invented target"
    );

    // Tick 3: this step's own npc is posted, five tiles away: the walk goes to
    // that posted row's own tile.
    let found = [talk_scene_npc(
        5,
        TANNER_ID,
        "Tanner",
        TileInput {
            x: 3200,
            z: 3205,
            level: 0,
        },
        5,
        &talk_to,
    )];
    post_scene(
        &iso,
        3,
        &page,
        &Scene {
            here: Some(TileInput {
                x: 3200,
                z: 3200,
                level: 0,
            }),
            npcs: &found,
            ..Scene::default()
        },
    );
    iso.on_game_tick(3);
    assert!(iso.probe("true").is_ok());
    let walked = iso.drain_interacts();
    assert_eq!(
        walked,
        vec![InteractReq::Walk {
            x: 3200,
            z: 3205,
            level: 0,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 0,
        }],
        "the walk goes to the posted match: {walked:?}"
    );

    // Tick 4: arrived beside it: the Talk-to is that posted row.
    let near = [talk_scene_npc(
        6,
        TANNER_ID,
        "Tanner",
        TileInput {
            x: 3200,
            z: 3205,
            level: 0,
        },
        1,
        &talk_to,
    )];
    post_scene(
        &iso,
        4,
        &page,
        &Scene {
            here: Some(TileInput {
                x: 3200,
                z: 3205,
                level: 0,
            }),
            npcs: &near,
            ..Scene::default()
        },
    );
    iso.on_game_tick(4);
    assert!(iso.probe("true").is_ok());
    let talked = iso.drain_interacts();
    assert_eq!(
        talked,
        vec![InteractReq::Npc {
            name: "Tanner".to_string(),
            action: "Talk-to".to_string(),
            index: Some(6),
        }],
        "the Talk-to is the posted identity: {talked:?}"
    );

    // Tick 5: the probe tick — the arm has one posted step per call and no
    // completion of its own.
    post_scene(
        &iso,
        5,
        &page,
        &Scene {
            here: Some(TileInput {
                x: 3200,
                z: 3205,
                level: 0,
            }),
            npcs: &near,
            ..Scene::default()
        },
    );
    iso.on_game_tick(5);
    assert!(iso.probe("true").is_ok());
    let again = iso.drain_interacts();
    assert_eq!(
        again,
        vec![InteractReq::Npc {
            name: "Tanner".to_string(),
            action: "Talk-to".to_string(),
            index: Some(6),
        }],
        "the Talk-to repeats while the same step stays held: {again:?}"
    );

    // Tick 6: the probe tick.
    post_scene(
        &iso,
        6,
        &page,
        &Scene {
            here: Some(TileInput {
                x: 3200,
                z: 3205,
                level: 0,
            }),
            npcs: &near,
            ..Scene::default()
        },
    );
    iso.on_game_tick(6);
    assert!(iso.probe("true").is_ok());

    let probed = iso.probe("globalThis.__probe").unwrap();
    let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
    iso.join();
    let steps = value["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 9, "{value:?}");
    for (index, kind) in [
        (1, "callback.enabled"),
        (2, "callback.log"),
        (3, "callback.setStatus"),
        (4, "wait"),
        (5, "wait"),
        (6, "walk"),
        (7, "npc"),
        (8, "npc"),
    ] {
        assert_eq!(steps[index]["kind"], kind, "{index} {value:?}");
    }
    assert!(!value.to_string().contains("clue solved"), "{value:?}");
}

/// The public `api.clue.next` path over a challenge scroll: the page holds only
/// the selected `2842` scroll, the seam joins its parent talk step, the posted
/// count dialog is answered with the selected string as
/// `InteractReq::AnswerCount`, and an empty or zero-count page is still the
/// landed C3 refusal.
#[test]
fn v2_clue_challenge_scroll_joins_the_parent_and_answers_over_the_count_dialog() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__runs = (globalThis.__runs || 0) + 1;
  const runs = globalThis.__runs;
  if (runs === 1) {
    const begin = api.clue.begin();
    globalThis.__token = begin.ok ? begin.value.token : null;
    globalThis.__steps = [
      begin,
      api.clue.next({ token: globalThis.__token }),
      api.clue.next({ token: globalThis.__token, resume: true }),
      api.clue.next({ token: globalThis.__token }),
      // The parent's own step, behind the posted count dialog.
      api.clue.next({ token: globalThis.__token }),
    ];
    return;
  }
  if (runs === 2) {
    globalThis.__steps.push(api.clue.next({ token: globalThis.__token }));
    return;
  }
  if (runs === 3) {
    globalThis.__steps.push(api.clue.next({ token: globalThis.__token }));
    return;
  }
  if (runs === 4) {
    globalThis.__emptyBegin = api.clue.begin();
    return;
  }
  globalThis.__zeroBegin = api.clue.begin();
  globalThis.__probe = JSON.stringify({
    token: globalThis.__token,
    runs: runs,
    steps: globalThis.__steps,
    emptyBegin: globalThis.__emptyBegin,
    zeroBegin: globalThis.__zeroBegin,
  });
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso =
        LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data).unwrap();
    let page = [(CHALLENGE_ID, 1)];
    let talk_to = vec!["Talk-to".to_string()];

    // Tick 1: the scroll alone. The identify is `none-held`, the seam hands out
    // the parent's token, and the posted count dialog takes the answer.
    post_scene(
        &iso,
        1,
        &page,
        &Scene {
            here: Some(TileInput {
                x: 2678,
                z: 3086,
                level: 1,
            }),
            count_dialog_open: true,
            ..Scene::default()
        },
    );
    iso.on_game_tick(1);
    assert!(iso.probe("true").is_ok());
    let answered = iso.drain_interacts();
    assert_eq!(
        answered,
        vec![InteractReq::AnswerCount {
            value: CHALLENGE_ANSWER,
        }],
        "the count dialog takes the selected answer: {answered:?}"
    );

    // Tick 2: the dialog is closed and the parent's npc is posted on the
    // published plane-1 tile: the step is still the parent's own talk step.
    let hazelmere = [talk_scene_npc(
        7,
        669,
        "Hazelmere",
        TileInput {
            x: 2678,
            z: 3086,
            level: 1,
        },
        1,
        &talk_to,
    )];
    post_scene(
        &iso,
        2,
        &page,
        &Scene {
            here: Some(TileInput {
                x: 2678,
                z: 3086,
                level: 1,
            }),
            npcs: &hazelmere,
            ..Scene::default()
        },
    );
    iso.on_game_tick(2);
    assert!(iso.probe("true").is_ok());
    let talked = iso.drain_interacts();
    assert_eq!(
        talked,
        vec![InteractReq::Npc {
            name: "Hazelmere".to_string(),
            action: "Talk-to".to_string(),
            index: Some(7),
        }],
        "the parent's own Talk-to: {talked:?}"
    );

    // Tick 3: an empty page ends the session with the landed refusal.
    post_page(&iso, 3, &[]);
    iso.on_game_tick(3);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "a refused session has no verbs"
    );

    // Tick 4: a begin on the empty page is refused the same way.
    post_page(&iso, 4, &[]);
    iso.on_game_tick(4);
    assert!(iso.probe("true").is_ok());
    assert!(iso.drain_interacts().is_empty(), "a refusal pushes nothing");

    // Tick 5: a zero count is not a held scroll: still `none-held`.
    post_page(&iso, 5, &[(CHALLENGE_ID, 0)]);
    iso.on_game_tick(5);
    assert!(iso.probe("true").is_ok());
    assert!(
        iso.drain_interacts().is_empty(),
        "a zero count is no scroll"
    );

    let probed = iso.probe("globalThis.__probe").unwrap();
    let value: serde_json::Value = serde_json::from_str(probed.as_str().unwrap()).unwrap();
    iso.join();
    let steps = value["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 7, "{value:?}");
    assert_eq!(steps[0]["ok"], true, "{value:?}");
    assert!(steps[0]["value"]["token"].is_number(), "{value:?}");
    for (index, kind) in [
        (1, "callback.enabled"),
        (2, "callback.log"),
        (3, "callback.setStatus"),
        (4, "answer-count"),
        (5, "npc"),
    ] {
        assert_eq!(steps[index]["kind"], kind, "{index} {value:?}");
    }
    assert_eq!(steps[4]["value"], CHALLENGE_ANSWER, "{value:?}");
    // The step is the parent's: the log line names its alias and its id, and
    // never the scroll id.
    let logged = steps[2]["message"].as_str().unwrap_or("");
    assert!(logged.contains("trail_clue_medium_anagram001"), "{value:?}");
    assert!(
        logged.contains(&TALK_CHALLENGE_ID.to_string()),
        "{value:?}"
    );
    assert!(!logged.contains(&CHALLENGE_ID.to_string()), "{value:?}");
    assert_eq!(steps[6]["ok"], false, "{value:?}");
    assert_eq!(steps[6]["error"], "none-held", "{value:?}");
    for refusal in ["emptyBegin", "zeroBegin"] {
        assert_eq!(value[refusal]["ok"], false, "{refusal} {value:?}");
        assert_eq!(value[refusal]["error"], "none-held", "{refusal} {value:?}");
    }
    assert!(!value.to_string().contains("clue solved"), "{value:?}");
}
