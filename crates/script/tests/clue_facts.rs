//! Clue rows over the landed schema-4 trail membership family, plus the v2
//! clue machine's live host path over posted snapshots.
use api::clue_facts::CluePin;
use client::io::ClientRevision;
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
        let casket = row(
            revision,
            CluePin::Alias("trail_clue_hard_sextant028_casket"),
        )
        .unwrap();
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
        for id in [
            3533,
            2842,
            2844,
            2846,
            2850,
            2852,
            2854,
            6859,
            0,
            -1,
            i32::MAX,
        ] {
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
        assert_eq!(
            row(revision, CluePin::Id(3554)).unwrap()["alias"],
            "trail_clue_hard_sextant028"
        );
    }
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
    /// The posted chat choices the trio acquire chain reads, in posted order:
    /// the row's own 1-based slot is its position here.
    chat_options: &'a [script::isolate_fb::ChatOptionInput<'a>],
    /// Whether the posted pack carries the held coordinate trio: a sextant
    /// row's own acquire chain owns the row until it does, so every dig scene
    /// in this file posts it.
    trio: bool,
    /// The posted count dialog: the talk arm answers only behind a posted
    /// `true`, so this is what the challenge step's answer rides.
    count_dialog_open: bool,
    /// The posted walk outcome's navigator-named gate shorts: the strict
    /// route's own `Carry` diagnosis. Empty is the observed "this outcome
    /// names no short" — the vector rides the walk outcome family, whose posted
    /// seq every keyframe here carries — and a page that names no short is
    /// never a shopping list.
    missing_carry: &'a [script::isolate_fb::CarryInput<'a>],
    /// The posted shop interface the gate-toll trip reads: the interface flag
    /// and the stock rows the buy click's own identity rides.
    shop_open: bool,
    shop_stock: &'a [script::isolate_fb::ItemRowInput<'a>],
    /// The posted worn page the Entrana strip reads: the raw
    /// `host().snapshot.equipment` rows with their own slot.
    equipment: &'a [script::isolate_fb::ItemRowInput<'a>],
    /// The posted nearest Use-quickly booth the strip's and the restore's bank
    /// trip opens.
    nearest_booth: Option<&'a script::isolate_fb::NearestBoothInput<'a>>,
    /// The posted bank interface: the strip's deposit and the restore's claim
    /// go out only behind a posted open one.
    bank_open: bool,
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
            chat_options: &[],
            trio: false,
            count_dialog_open: false,
            missing_carry: &[],
            shop_open: false,
            shop_stock: &[],
            equipment: &[],
            nearest_booth: None,
            bank_open: false,
        }
    }
}
/// One posted loc row with the tile, id and actions the native snapshot
/// carries into the clue machine.
fn scene_loc<'a>(
    id: i32,
    x: i32,
    z: i32,
    level: i32,
    actions: &'a [String],
) -> script::isolate_fb::SceneEntityInput<'a> {
    script::isolate_fb::SceneEntityInput {
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

/// Selected coordinate-tool rows added to fixtures that exercise trio-owned steps.
const TRIO_ITEMS: [(i32, &str); 3] = [(2574, "Sextant"), (2575, "Watch"), (2576, "Chart")];

/// Every fixture posts the same walk outcome generation.
const POSTED_WALK_OUTCOME_SEQ: u64 = 1;

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
    let mut held: Vec<(i32, i32)> = page.to_vec();
    if scene.trio {
        held.extend(TRIO_ITEMS.iter().map(|(id, _)| (*id, 1)));
    }
    let rows: Vec<script::isolate_fb::ItemRowInput<'_>> = held
        .iter()
        .map(|(id, count)| script::isolate_fb::ItemRowInput {
            name: scene
                .names
                .iter()
                .find(|(row_id, _)| row_id == id)
                .map(|(_, name)| *name)
                .or_else(|| {
                    TRIO_ITEMS
                        .iter()
                        .find(|(row_id, _)| row_id == id)
                        .map(|(_, name)| *name)
                }),
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
        bank_open: scene.bank_open,
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
        equipment: scene.equipment,
        chat_open: false,
        chat_continue: scene.chat_continue,
        chat_text: None,
        chat_options: scene.chat_options,
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
        nearest_booth: scene
            .nearest_booth
            .map(|booth| script::isolate_fb::NearestBoothInput {
                x: booth.x,
                z: booth.z,
                level: booth.level,
                id: booth.id,
                name: booth.name,
                op: booth.op,
            }),
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
        shop_open: scene.shop_open,
        shop_stock: scene.shop_stock,
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
        // The family's own seq: it is what makes the posted carry vector above
        // a family post at all, and every keyframe here carries it so a page
        // that names a short is observed as one.
        walk_outcome_seq: POSTED_WALK_OUTCOME_SEQ,
        walk_missing_carry: scene.missing_carry,
        ..script::isolate_fb::NativeFactsInput::default()
    };
    iso.post_snapshot(script::isolate_fb::encode_snapshot_with_native(
        &input, native,
    ));
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
    run: typeof api.clue.run,
    packPlan: typeof api.clue.packPlan,
    hardKit: typeof api.clue.hardKit,
    keep: typeof api.clue.keep,
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
        for key in ["begin", "run"] {
            assert_eq!(value[key], "function", "{key} {value:?}");
        }
        assert_eq!(
            value["retry"], "function",
            "the landed latch clear, after a run: {value:?}"
        );
        for key in ["challengeAnswer", "deposit", "noteDeath"] {
            assert_eq!(value[key], "undefined", "{key} {value:?}");
        }
        assert_eq!(value["packPlan"], "function", "{value:?}");
        assert_eq!(value["hardKit"], "function", "{value:?}");
        assert_eq!(value["keep"], "function", "{value:?}");
        assert_eq!(value["quest"], "undefined", "{value:?}");
        assert_eq!(
            value["questionNamespace"],
            serde_json::json!([
                "row", "heldStep", "packPlan", "hardKit", "keep", "begin", "run", "retry"
            ]),
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
    assert_eq!(
        value["stringAlias"]["value"]["access"], "constrained",
        "{value:?}"
    );
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
        assert_eq!(
            value["alias"], "trail_clue_hard_sextant016_casket",
            "{value:?}"
        );
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
fn v2_clue_run_without_hooks_defaults_enabled_true_and_progresses() {
    let src = r#"
export const apiVersion = 2;
let started = false;
export async function tick(api) {
  if (started) return;
  started = true;
  const begin = api.clue.begin();
  globalThis.__begin = begin;
  globalThis.__out = null;
  globalThis.__error = null;
  try {
    globalThis.__out = await api.clue.run(begin.value);
  } catch (error) {
    globalThis.__error = String(error && error.message ? error.message : error);
  }
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso =
        LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data).unwrap();

    post_page(&iso, 1, &[(2677, 1)]);
    iso.on_game_tick(1);
    assert_eq!(iso.probe("globalThis.__begin.ok").unwrap(), true);
    assert!(iso.probe("globalThis.__out").unwrap().is_null());
    assert!(iso.probe("globalThis.__error").unwrap().is_null());
    assert!(iso.drain_interacts().is_empty());

    post_scene(
        &iso,
        2,
        &[(2677, 1)],
        &Scene {
            here: Some(script::isolate_fb::TileInput {
                x: 3200,
                z: 3218,
                level: 1,
            }),
            ..Scene::default()
        },
    );
    iso.on_game_tick(2);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Walk {
            x: 3209,
            z: 3218,
            level: 1,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 0,
        }],
        "an absent enabled hook defaults true and advances into the search arm"
    );
    assert!(iso.probe("globalThis.__out").unwrap().is_null());

    post_scene(
        &iso,
        3,
        &[(2677, 1)],
        &Scene {
            ours: true,
            ..Scene::default()
        },
    );
    iso.on_game_tick(3);
    assert!(iso.probe("true").is_ok());
    let out = iso.probe("globalThis.__out").unwrap();
    assert_eq!(out["kind"], "done", "{out:?}");
    assert_eq!(out["value"]["kind"], "yield", "{out:?}");
    assert!(iso.probe("globalThis.__error").unwrap().is_null());
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}
/// The awaited public runner owns the callback replies and maps Rust verbs over
/// live snapshots. Missing observations remain pending waits; a terminal kind
/// settles the one promise.
#[test]
fn v2_clue_run_invokes_hooks_and_maps_wait_walk_loc_and_terminal_yield() {
    let src = r#"
export const apiVersion = 2;
let started = false;
export async function tick(api) {
  if (started) return;
  started = true;
  const begin = api.clue.begin();
  globalThis.__begin = begin;
  globalThis.__events = [];
  globalThis.__out = null;
  globalThis.__error = null;
  const hooks = {
    enabled() {
      globalThis.__events.push(['enabled', this === hooks]);
      return true;
    },
    log(message) {
      globalThis.__events.push(['log', message, this === hooks]);
    },
    setStatus(message) {
      globalThis.__events.push(['setStatus', message, this === hooks]);
    },
  };
  try {
    globalThis.__out = await api.clue.run(begin.value, hooks);
  } catch (error) {
    globalThis.__error = String(error && error.message ? error.message : error);
  }
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso =
        LoadIsolate::spawn_with_game_data(src.into(), LoadShape::NativeTick, vec![], data).unwrap();
    let page = [(2677, 1)];

    // No posted player tile: all three hooks answer in order, then the search
    // arm waits with no verb and the run stays pending.
    post_page(&iso, 1, &page);
    iso.on_game_tick(1);
    assert_eq!(iso.probe("globalThis.__begin.ok").unwrap(), true);
    let events = iso.probe("globalThis.__events").unwrap();
    let events = events.as_array().expect("hook events");
    assert_eq!(events.len(), 3, "{events:?}");
    assert_eq!(events[0], serde_json::json!(["enabled", true]));
    assert_eq!(events[1][0], "log", "{events:?}");
    assert_eq!(events[1][2], true, "{events:?}");
    assert!(
        events[1][1]
            .as_str()
            .unwrap_or("")
            .contains("trail_clue_easy_simple001"),
        "{events:?}"
    );
    assert_eq!(events[2][0], "setStatus", "{events:?}");
    assert_eq!(events[2][2], true, "{events:?}");
    assert!(iso.probe("globalThis.__out").unwrap().is_null());
    assert!(iso.probe("globalThis.__error").unwrap().is_null());
    assert!(iso.drain_interacts().is_empty());

    // A posted tile away from the decoded destination maps the machine's walk
    // into the host interact queue.
    post_scene(
        &iso,
        2,
        &page,
        &Scene {
            here: Some(script::isolate_fb::TileInput {
                x: 3200,
                z: 3218,
                level: 1,
            }),
            ..Scene::default()
        },
    );
    iso.on_game_tick(2);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Walk {
            x: 3209,
            z: 3218,
            level: 1,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 0,
        }]
    );
    assert!(iso.probe("globalThis.__out").unwrap().is_null());

    // Arrived with one posted searchable loc: the exact posted row is mapped.
    let search = vec!["Search".to_string()];
    let locs = [scene_loc(25, 3210, 3217, 1, &search)];
    post_scene(
        &iso,
        3,
        &page,
        &Scene {
            here: Some(script::isolate_fb::TileInput {
                x: 3209,
                z: 3218,
                level: 1,
            }),
            locs: &locs,
            ..Scene::default()
        },
    );
    iso.on_game_tick(3);
    assert!(iso.probe("true").is_ok());
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Loc {
            x: 3210,
            z: 3217,
            level: 1,
            action: "Search".to_string(),
            id: Some(25),
        }]
    );
    assert!(iso.probe("globalThis.__out").unwrap().is_null());

    // Arrived with no searchable loc: another pending wait, not an invented
    // interaction or a terminal.
    post_scene(
        &iso,
        4,
        &page,
        &Scene {
            here: Some(script::isolate_fb::TileInput {
                x: 3209,
                z: 3218,
                level: 1,
            }),
            ..Scene::default()
        },
    );
    iso.on_game_tick(4);
    assert!(iso.probe("true").is_ok());
    assert!(iso.drain_interacts().is_empty());
    assert!(iso.probe("globalThis.__out").unwrap().is_null());

    // The posted cooperative interrupt is the terminal machine kind and
    // settles the public run envelope.
    post_scene(
        &iso,
        5,
        &page,
        &Scene {
            ours: true,
            ..Scene::default()
        },
    );
    iso.on_game_tick(5);
    let out = iso.probe("globalThis.__out").unwrap();
    assert_eq!(out["kind"], "done", "{out:?}");
    assert_eq!(out["value"]["kind"], "yield", "{out:?}");
    assert!(iso.probe("globalThis.__error").unwrap().is_null());
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

const LIVE_CLUE_RUN: &str = r#"
export const apiVersion = 2;
let started = false;
export async function tick(api) {
  if (started) return;
  started = true;
  const begin = api.clue.begin();
  globalThis.__begin = begin;
  globalThis.__out = null;
  globalThis.__error = null;
  try {
    globalThis.__out = await api.clue.run(begin.value);
  } catch (error) {
    globalThis.__error = String(error && error.message ? error.message : error);
  }
}
"#;

fn spawn_live_clue_run() -> LoadIsolate {
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    LoadIsolate::spawn_with_game_data(LIVE_CLUE_RUN.into(), LoadShape::NativeTick, vec![], data)
        .unwrap()
}

fn live_clue_tick(
    iso: &LoadIsolate,
    tick: u64,
    page: &[(i32, i32)],
    scene: &Scene<'_>,
) -> Vec<InteractReq> {
    post_scene(iso, tick, page, scene);
    iso.on_game_tick(tick);
    assert!(iso.probe("true").is_ok());
    iso.drain_interacts()
}

fn assert_live_yield(iso: &LoadIsolate) {
    let out = iso.probe("globalThis.__out").unwrap();
    assert_eq!(out["kind"], "done", "{out:?}");
    assert_eq!(out["value"]["kind"], "yield", "{out:?}");
    assert!(iso.probe("globalThis.__error").unwrap().is_null());
}

fn clue_walk(x: i32, z: i32, level: i32) -> InteractReq {
    InteractReq::Walk {
        x,
        z,
        level,
        allow_teleports: false,
        allow_wilderness: false,
        allow_bank_fetch: false,
        request_id: 0,
    }
}

fn clue_npc<'a>(
    index: i32,
    id: i32,
    name: &'a str,
    tile: script::isolate_fb::TileInput,
    distance: i32,
    actions: &'a [String],
) -> script::isolate_fb::SceneEntityInput<'a> {
    script::isolate_fb::SceneEntityInput {
        index,
        id,
        name: Some(name),
        x: tile.x,
        z: tile.z,
        level: tile.level,
        distance,
        health: 10,
        max_health: 10,
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

fn clue_stock(
    id: i32,
    name: &str,
    slot: i32,
    component: i32,
) -> script::isolate_fb::ItemRowInput<'_> {
    script::isolate_fb::ItemRowInput {
        name: Some(name),
        count: 500,
        id,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: component,
        slot,
    }
}

#[test]
fn v2_clue_run_reads_gate_toll_scene_and_shops_once() {
    const PASS: i32 = 1854;
    const SHANTAY: i32 = 836;
    const DEST: (i32, i32, i32) = (3209, 3218, 1);
    const SPAWN: (i32, i32, i32) = (3304, 3123, 0);
    let iso = spawn_live_clue_run();
    let page = [(2677, 1)];
    let packed = [(2677, 1), (PASS, 1)];
    let named = [script::isolate_fb::CarryInput {
        id: PASS,
        count: 1,
        name: Some("Shantay pass"),
    }];
    let stock = [clue_stock(PASS, "Shantay pass", 16, 3900)];
    let pack_names = [(PASS, "Shantay pass")];
    let trade = ["Trade".to_string()];
    let keeper = [clue_npc(
        12,
        SHANTAY,
        "Shantay",
        script::isolate_fb::TileInput {
            x: SPAWN.0,
            z: SPAWN.1,
            level: SPAWN.2,
        },
        1,
        &trade,
    )];
    let away = script::isolate_fb::TileInput {
        x: 3200,
        z: 3218,
        level: 1,
    };
    let spawn = script::isolate_fb::TileInput {
        x: SPAWN.0,
        z: SPAWN.1,
        level: SPAWN.2,
    };

    assert_eq!(
        live_clue_tick(
            &iso,
            1,
            &page,
            &Scene {
                here: Some(away),
                missing_carry: &named,
                ..Scene::default()
            },
        ),
        vec![clue_walk(DEST.0, DEST.1, DEST.2)]
    );
    assert_eq!(
        live_clue_tick(
            &iso,
            2,
            &page,
            &Scene {
                here: Some(away),
                missing_carry: &named,
                ..Scene::default()
            },
        ),
        vec![clue_walk(SPAWN.0, SPAWN.1, SPAWN.2)]
    );
    assert_eq!(
        live_clue_tick(
            &iso,
            3,
            &page,
            &Scene {
                here: Some(away),
                missing_carry: &named,
                ..Scene::default()
            },
        ),
        vec![clue_walk(SPAWN.0, SPAWN.1, SPAWN.2)]
    );
    assert_eq!(
        live_clue_tick(
            &iso,
            4,
            &page,
            &Scene {
                here: Some(spawn),
                npcs: &keeper,
                missing_carry: &named,
                ..Scene::default()
            },
        ),
        vec![InteractReq::Npc {
            name: "Shantay".to_string(),
            action: "Trade".to_string(),
            index: Some(12),
        }]
    );
    assert_eq!(
        live_clue_tick(
            &iso,
            5,
            &page,
            &Scene {
                here: Some(spawn),
                npcs: &keeper,
                missing_carry: &named,
                shop_open: true,
                shop_stock: &stock,
                ..Scene::default()
            },
        ),
        vec![InteractReq::ShopButton {
            kind: "buy".to_string(),
            name: "Shantay pass".to_string(),
            id: PASS,
            slot: 16,
            component: 3900,
            chunk: 1,
        }]
    );
    assert_eq!(
        live_clue_tick(
            &iso,
            6,
            &packed,
            &Scene {
                here: Some(spawn),
                npcs: &keeper,
                names: &pack_names,
                missing_carry: &named,
                shop_open: true,
                shop_stock: &stock,
                ..Scene::default()
            },
        ),
        vec![InteractReq::CloseModal]
    );
    assert_eq!(
        live_clue_tick(
            &iso,
            7,
            &packed,
            &Scene {
                here: Some(spawn),
                names: &pack_names,
                missing_carry: &named,
                ..Scene::default()
            },
        ),
        vec![clue_walk(DEST.0, DEST.1, DEST.2)]
    );
    assert_eq!(
        live_clue_tick(
            &iso,
            8,
            &packed,
            &Scene {
                here: Some(spawn),
                names: &pack_names,
                missing_carry: &named,
                ..Scene::default()
            },
        ),
        vec![clue_walk(DEST.0, DEST.1, DEST.2)],
        "the held pass cannot start a second shop trip"
    );
    assert!(live_clue_tick(
        &iso,
        9,
        &packed,
        &Scene {
            ours: true,
            ..Scene::default()
        },
    )
    .is_empty());
    assert_live_yield(&iso);
    iso.join();
}

#[test]
fn v2_clue_run_reads_guarded_combat_scene_and_redigs_owned_death() {
    let iso = spawn_live_clue_run();
    let page = [(2723, 1), (952, 1)];
    let names = [(952, "Spade")];
    let attack = ["Attack".to_string()];
    let wizard_tile = script::isolate_fb::TileInput {
        x: 3058,
        z: 3884,
        level: 0,
    };
    let mut mine = clue_npc(7, 107, "Zamorak Wizard", wizard_tile, 3, &attack);
    mine.in_combat = true;
    mine.target_kind = 2;
    mine.target_index = 42;
    let decoy = clue_npc(8, 108, "Zamorak Wizard", wizard_tile, 1, &attack);
    let alive = [decoy, mine];
    let dead = [script::isolate_fb::SceneEntityInput {
        health: 0,
        max_health: 10,
        in_combat: true,
        target_kind: 2,
        target_index: 99,
        ..mine
    }];
    let stats = [script::isolate_fb::StatInput {
        index: 3,
        name: "hitpoints",
        xp: 0,
        base: 40,
        effective: 40,
    }];
    let overlay_off = [script::isolate_fb::VarpInput {
        index: 95,
        value: 0,
    }];
    let overlay_on = [script::isolate_fb::VarpInput {
        index: 95,
        value: 1,
    }];
    let far = script::isolate_fb::TileInput {
        x: 3100,
        z: 3300,
        level: 0,
    };
    let arrived = script::isolate_fb::TileInput {
        x: 3058,
        z: 3884,
        level: 0,
    };
    let dig = InteractReq::Held {
        name: "Spade".to_string(),
        action: "Dig".to_string(),
    };

    assert_eq!(
        live_clue_tick(
            &iso,
            1,
            &page,
            &Scene {
                here: Some(far),
                names: &names,
                stats: &stats,
                trio: true,
                ..Scene::default()
            },
        ),
        vec![clue_walk(3058, 3884, 0)]
    );
    assert_eq!(
        live_clue_tick(
            &iso,
            2,
            &page,
            &Scene {
                here: Some(arrived),
                names: &names,
                stats: &stats,
                trio: true,
                ..Scene::default()
            },
        ),
        vec![dig.clone()]
    );
    assert_eq!(
        live_clue_tick(
            &iso,
            3,
            &page,
            &Scene {
                here: Some(arrived),
                names: &names,
                npcs: &alive,
                varps: &overlay_off,
                stats: &stats,
                self_slot: 42,
                self_target_kind: 1,
                self_target_index: 7,
                trio: true,
                ..Scene::default()
            },
        ),
        vec![InteractReq::IfButton { component_id: 5621 }]
    );
    assert_eq!(
        live_clue_tick(
            &iso,
            4,
            &page,
            &Scene {
                here: Some(arrived),
                names: &names,
                npcs: &alive,
                varps: &overlay_on,
                stats: &stats,
                self_slot: 42,
                self_target_kind: 1,
                self_target_index: 7,
                trio: true,
                ..Scene::default()
            },
        ),
        vec![InteractReq::Npc {
            name: "Zamorak Wizard".to_string(),
            action: "Attack".to_string(),
            index: Some(7),
        }],
        "the farther wizard targeting self wins over the nearer decoy"
    );
    assert_eq!(
        live_clue_tick(
            &iso,
            5,
            &page,
            &Scene {
                here: Some(arrived),
                names: &names,
                npcs: &dead,
                varps: &overlay_on,
                stats: &stats,
                self_slot: 42,
                self_target_kind: 1,
                self_target_index: 7,
                trio: true,
                ..Scene::default()
            },
        ),
        vec![dig],
        "zero health with a positive maximum and the self target pair proves the owned kill"
    );
    assert!(live_clue_tick(
        &iso,
        6,
        &page,
        &Scene {
            ours: true,
            ..Scene::default()
        },
    )
    .is_empty());
    assert_live_yield(&iso);
    iso.join();
}

const PUZZLE_PIECE_B: i32 = 2749;
const PUZZLE_COMPONENT: i32 = 6600;

fn puzzle_rows(solved: bool) -> Vec<script::isolate_fb::ItemRowInput<'static>> {
    let mut rows = Vec::with_capacity(24);
    for slot in 0..25 {
        let target = if solved {
            (slot != 24).then_some(slot)
        } else if slot == 24 {
            Some(23)
        } else if slot == 23 {
            None
        } else {
            Some(slot)
        };
        if let Some(target) = target {
            rows.push(script::isolate_fb::ItemRowInput {
                name: Some("Sliding piece"),
                count: 1,
                id: PUZZLE_PIECE_B + target,
                ops: &[],
                noted: false,
                cert: -1,
                component_id: PUZZLE_COMPONENT,
                slot,
            });
        }
    }
    rows
}

#[test]
fn v2_clue_run_reads_puzzle_board_and_generation() {
    let iso = spawn_live_clue_run();
    let page = [(2794, 1), (2795, 1)];
    assert_eq!(
        live_clue_tick(&iso, 1, &page, &Scene::default()),
        vec![InteractReq::Held {
            name: "Puzzle box".to_string(),
            action: "Open".to_string(),
        }]
    );

    let one_move = puzzle_rows(false);
    let one_move_scene = Scene {
        puzzle: Some(PostedPuzzle {
            component_id: PUZZLE_COMPONENT,
            size: 25,
            items: &one_move,
            generation: 7,
        }),
        ..Scene::default()
    };
    assert_eq!(
        live_clue_tick(&iso, 2, &page, &one_move_scene),
        vec![InteractReq::PuzzleMove {
            id: PUZZLE_PIECE_B + 23,
            slot: 24,
            component: PUZZLE_COMPONENT,
            generation: 7,
        }]
    );

    let solved = puzzle_rows(true);
    let solved_scene = Scene {
        puzzle: Some(PostedPuzzle {
            component_id: PUZZLE_COMPONENT,
            size: 25,
            items: &solved,
            generation: 8,
        }),
        ..Scene::default()
    };
    assert_eq!(
        live_clue_tick(&iso, 3, &page, &solved_scene),
        vec![InteractReq::CloseModal]
    );
    assert!(
        live_clue_tick(&iso, 4, &page, &solved_scene).is_empty(),
        "the solved board is not closed twice"
    );
    assert!(live_clue_tick(
        &iso,
        5,
        &page,
        &Scene {
            ours: true,
            ..Scene::default()
        },
    )
    .is_empty());
    assert_live_yield(&iso);
    iso.join();
}
