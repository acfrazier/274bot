//! Quest query over the landed schema-4 family. Not a tool table. Not a prayer machine.
use api::quest_facts::IdentityPin;
use client::io::ClientRevision;
use script::{LoadIsolate, LoadShape};

fn facts(revision: ClientRevision) -> api::game_data::QuestIdentityFacts {
    api::game_data::for_revision(revision)
        .unwrap()
        .quest_identity()
        .expect("landed family")
        .clone()
}

fn identity(
    revision: ClientRevision,
    pin: IdentityPin<'_>,
) -> Result<serde_json::Value, &'static str> {
    let facts = facts(revision);
    api::quest_facts::quest_identity(Some(&facts), pin)
}

fn prereqs(revision: ClientRevision, id: &str) -> Result<serde_json::Value, &'static str> {
    let facts = facts(revision);
    api::quest_facts::quest_prereqs(Some(&facts), id)
}

#[test]
fn missing_family_is_the_method_token_before_the_key() {
    let blank = api::quest_facts::quest_identity(None, IdentityPin::Id("   "));
    assert_eq!(blank, Err("family-unavailable:quest_identity"));
    let would_hit = api::quest_facts::quest_identity(None, IdentityPin::Id("death"));
    assert_eq!(would_hit, Err("family-unavailable:quest_identity"));
    let miss = api::quest_facts::quest_identity(None, IdentityPin::Name("nope"));
    assert_eq!(miss, Err("family-unavailable:quest_identity"));
    let prereq_blank = api::quest_facts::quest_prereqs(None, "   ");
    assert_eq!(prereq_blank, Err("family-unavailable:quest_prereqs"));
    let prereq_hit = api::quest_facts::quest_prereqs(None, "cook");
    assert_eq!(prereq_hit, Err("family-unavailable:quest_prereqs"));
    assert_ne!(
        api::quest_facts::quest_identity(None, IdentityPin::Id("death")),
        Ok(serde_json::json!({}))
    );
    assert_ne!(
        api::quest_facts::quest_prereqs(None, "cook"),
        Ok(serde_json::json!({ "skills": [], "items": [] }))
    );
    assert_ne!(blank, Err("family-unavailable:gather_methods"));
}

#[test]
fn id_matches_seed_id_only_and_name_matches_display_only() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let death = identity(revision, IdentityPin::Id("  DeAtH  ")).unwrap();
        assert!(death.get("rows").is_none(), "{death}");
        assert!(death.get("engine_id").is_none(), "{death}");
        assert!(death.get("coverage").is_none(), "{death}");
        assert!(death.get("qualification").is_none(), "{death}");
        assert_eq!(death["id"], "death");
        assert_eq!(death["component"], "death");
        assert_eq!(death["display"], "Death Plateau");
        assert_eq!(death["varp"], "death_equiproom");
        assert_eq!(death["varp_id"], 314);
        assert_eq!(death["complete"], 80);
        assert!(death["complete"].is_number());
        assert_eq!(death["quest_points"], 1);
        assert_eq!(death["unknown_sides"], serde_json::json!([]));
        assert_eq!(death["requirements"]["qualification"], "partial");
        assert_eq!(death["requirements"]["unknown_as_satisfied"], false);
        assert_eq!(death["requirements"]["empty_must_have"], true);
        assert_eq!(death["requirements"]["items"], serde_json::json!([]));

        let by_name = identity(revision, IdentityPin::Name("  death plateau  ")).unwrap();
        assert_eq!(by_name, death);
        assert_eq!(
            identity(revision, IdentityPin::Name("death")),
            Err("unknown-quest")
        );
        for miss in [
            "DeathPlateau",
            "death_plateau",
            "cookquest",
            "murderquest",
            "waterfall_quest",
            "death_equiproom",
            "314",
            "219",
            "Rune Mysteries",
            "Waterfall",
            "Lost City Of Zanaris",
            "routequest",
            "In Search of the Myreque",
            "misc",
            "troll_love",
            "mm",
            "",
            "   ",
            "Death  Plateau",
        ] {
            assert_eq!(
                identity(revision, IdentityPin::Name(miss)),
                Err("unknown-quest"),
                "{revision:?} name {miss:?}"
            );
        }
        for miss in [
            "cookquest",
            "murderquest",
            "waterfall_quest",
            "death_equiproom",
            "Death Plateau",
            "314",
            "219",
            "routequest",
            "In Search of the Myreque",
            "misc",
            "troll_love",
            "mm",
            "Cook's Assistant",
            "",
            "   ",
        ] {
            assert_eq!(
                identity(revision, IdentityPin::Id(miss)),
                Err("unknown-quest"),
                "{revision:?} id {miss:?}"
            );
        }
    }
}

#[test]
fn identity_value_is_the_landed_row_on_both_pins() {
    let points = [
        ("cook", 1, 2, 29, "cookquest"),
        ("runemysteries", 1, 6, 63, "runemysteries"),
        ("murder", 3, 2, 192, "murderquest"),
        ("waterfall", 1, 10, 65, "waterfall_quest"),
        ("death", 1, 80, 314, "death_equiproom"),
        ("zanaris", 3, 6, 147, "zanaris"),
    ];
    let left = identity(ClientRevision::R274, IdentityPin::Id("cook")).unwrap();
    let right = identity(ClientRevision::R289, IdentityPin::Id("COOK")).unwrap();
    assert_eq!(left, right);
    for (id, points, complete, varp_id, varp) in points {
        for revision in [ClientRevision::R274, ClientRevision::R289] {
            let row = identity(revision, IdentityPin::Id(id)).unwrap();
            assert_eq!(row["quest_points"], points, "{id}");
            assert_eq!(row["complete"], complete, "{id}");
            assert_eq!(row["varp_id"], varp_id, "{id}");
            assert_eq!(row["varp"], varp, "{id}");
            assert_eq!(row["requirements"]["qualification"], "partial");
            assert_eq!(row["requirements"]["unknown_as_satisfied"], false);
            assert!(row.get("engine_id").is_none());
            assert!(row["complete"].as_object().is_none());
        }
    }
    let cook = identity(ClientRevision::R274, IdentityPin::Name("Cook's Assistant")).unwrap();
    assert_eq!(cook["id"], "cook");
    assert_eq!(
        cook["requirements"]["items"],
        serde_json::json!([
            { "alias": "egg", "quantity": 1, "kind": "inv" },
            { "alias": "bucket_milk", "quantity": 1, "kind": "inv" },
            { "alias": "pot_flour", "quantity": 1, "kind": "inv" }
        ])
    );
    let waterfall = identity(ClientRevision::R289, IdentityPin::Name("Waterfall Quest")).unwrap();
    assert_eq!(
        waterfall["requirements"]["items"][0],
        serde_json::json!({ "alias": "rope", "quantity": null, "kind": "use-site" })
    );
    let zanaris = identity(ClientRevision::R274, IdentityPin::Id("zanaris")).unwrap();
    assert_eq!(
        zanaris["requirements"]["skills"],
        serde_json::json!([
            { "skill": "woodcutting", "level": 36 },
            { "skill": "crafting", "level": 31 }
        ])
    );
    assert_eq!(zanaris["display"], "Lost City");
    assert_eq!(
        identity(ClientRevision::R274, IdentityPin::Name("Lost City")).unwrap()["id"],
        "zanaris"
    );
}

#[test]
fn prereqs_value_is_the_requirements_object_only() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let cook = prereqs(revision, "  cook  ").unwrap();
        assert!(cook.get("id").is_none(), "{cook}");
        assert!(cook.get("coverage").is_none(), "{cook}");
        assert!(cook.get("rows").is_none(), "{cook}");
        assert!(cook.get("display").is_none(), "{cook}");
        assert_eq!(cook["qualification"], "partial");
        assert_eq!(cook["unknown_as_satisfied"], false);
        assert_eq!(cook["empty_must_have"], false);
        assert_eq!(
            cook["items"],
            serde_json::json!([
                { "alias": "egg", "quantity": 1, "kind": "inv" },
                { "alias": "bucket_milk", "quantity": 1, "kind": "inv" },
                { "alias": "pot_flour", "quantity": 1, "kind": "inv" }
            ])
        );
        assert!(cook["items"][0].get("Egg").is_none());
        let rope = prereqs(revision, "waterfall").unwrap();
        assert_eq!(rope["items"][0]["quantity"], serde_json::Value::Null);
        assert_eq!(rope["items"][0]["alias"], "rope");
        assert_eq!(rope["items"][0]["kind"], "use-site");
        for id in ["runemysteries", "murder", "death"] {
            let empty = prereqs(revision, id).unwrap();
            assert_eq!(empty["ok"], serde_json::Value::Null);
            assert_eq!(empty["qualification"], "partial", "{id}");
            assert_eq!(empty["unknown_as_satisfied"], false, "{id}");
            assert_eq!(empty["empty_must_have"], true, "{id}");
            assert_eq!(empty["items"], serde_json::json!([]), "{id}");
            assert_eq!(empty["skills"], serde_json::json!([]), "{id}");
        }
        let zanaris = prereqs(revision, "ZANARIS").unwrap();
        assert_eq!(zanaris["skills"][0]["skill"], "woodcutting");
        assert_eq!(zanaris["skills"][0]["level"], 36);
        assert_eq!(zanaris["skills"][1]["skill"], "crafting");
        assert_eq!(zanaris["skills"][1]["level"], 31);
        for miss in [
            "Cook's Assistant",
            "routequest",
            "In Search of the Myreque",
            "misc",
            "troll_love",
            "mm",
            "314",
            "219",
            "death_equiproom",
            "",
            "   ",
        ] {
            assert_eq!(
                prereqs(revision, miss),
                Err("unknown-quest"),
                "{revision:?} {miss:?}"
            );
        }
    }
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
    std::sync::Arc::new(serde_json::from_str(raw).expect("selected data without quest_identity"))
}

#[test]
fn v2_identity_and_prereqs_are_sync_helper_results() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const death = api.questIdentity({ id: 'death' });
  const named = api.questIdentity({ name: 'Death Plateau' });
  const cook = api.questPrereqs({ id: 'cook', name: "Cook's Assistant" });
  const empty = api.questPrereqs({ id: 'death' });
  globalThis.__probe = JSON.stringify({
    deathOk: death.ok,
    deathThen: typeof death.then,
    deathId: death.value && death.value.id,
    deathVarp: death.value && death.value.varp,
    deathVarpId: death.value && death.value.varp_id,
    deathComplete: death.value && death.value.complete,
    deathPoints: death.value && death.value.quest_points,
    deathRows: death.value && death.value.rows,
    deathEngine: death.value && death.value.engine_id,
    namedId: named.value && named.value.id,
    cookOk: cook.ok,
    cookQual: cook.value && cook.value.qualification,
    cookEgg: cook.value && cook.value.items && cook.value.items[0].alias,
    cookId: cook.value && cook.value.id,
    cookCoverage: cook.value && cook.value.coverage,
    emptyQual: empty.value && empty.value.qualification,
    emptyMust: empty.value && empty.value.empty_must_have,
    emptyUnknown: empty.value && empty.value.unknown_as_satisfied,
    emptyItems: empty.value && empty.value.items.length,
    omitted: api.questIdentity(),
    emptyArg: api.questIdentity({}),
    both: api.questIdentity({ id: 'cook', name: "Cook's Assistant" }),
    array: api.questIdentity([]),
    number: api.questIdentity({ id: 314 }),
    string314: api.questIdentity({ id: '314' }),
    blank: api.questIdentity({ id: '   ' }),
    nameDeath: api.questIdentity({ name: 'death' }),
    route: api.questIdentity({ id: 'routequest' }),
    prereqOmitted: api.questPrereqs(),
    prereqEmpty: api.questPrereqs({}),
    prereqName: api.questPrereqs({ name: 'cook' }),
    prereqDisplay: api.questPrereqs({ id: "Cook's Assistant" }),
    promise: death instanceof Promise,
    namespace: api.quest,
  });
}
"#;
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let value = probe(src, revision);
        assert_eq!(value["deathOk"], true, "{value:?}");
        assert_eq!(value["deathThen"], "undefined");
        assert_eq!(value["deathId"], "death");
        assert_eq!(value["deathVarp"], "death_equiproom");
        assert_eq!(value["deathVarpId"], 314);
        assert_eq!(value["deathComplete"], 80);
        assert_eq!(value["deathPoints"], 1);
        assert!(value["deathRows"].is_null(), "{value:?}");
        assert!(value["deathEngine"].is_null(), "{value:?}");
        assert_eq!(value["namedId"], "death");
        assert_eq!(value["cookOk"], true, "{value:?}");
        assert_eq!(value["cookQual"], "partial");
        assert_eq!(value["cookEgg"], "egg");
        assert!(value["cookId"].is_null(), "{value:?}");
        assert!(value["cookCoverage"].is_null(), "{value:?}");
        assert_eq!(value["emptyQual"], "partial");
        assert_eq!(value["emptyMust"], true);
        assert_eq!(value["emptyUnknown"], false);
        assert_eq!(value["emptyItems"], 0);
        assert_eq!(value["omitted"]["error"], "invalid-args");
        assert_eq!(value["emptyArg"]["error"], "invalid-args");
        assert_eq!(value["both"]["error"], "invalid-args");
        assert_eq!(value["array"]["error"], "invalid-args");
        assert_eq!(value["number"]["error"], "invalid-args");
        assert_eq!(value["string314"]["error"], "unknown-quest");
        assert_eq!(value["blank"]["error"], "unknown-quest");
        assert_eq!(value["nameDeath"]["error"], "unknown-quest");
        assert_eq!(value["route"]["error"], "unknown-quest");
        assert_eq!(value["prereqOmitted"]["error"], "invalid-args");
        assert_eq!(value["prereqEmpty"]["error"], "invalid-args");
        assert_eq!(value["prereqName"]["error"], "invalid-args");
        assert_eq!(value["prereqDisplay"]["error"], "unknown-quest");
        assert_eq!(value["promise"], false);
        assert!(value["namespace"].is_null());
    }
}

#[test]
fn v2_invalid_args_before_missing_slot_and_missing_family() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__probe = JSON.stringify({
    omitted: api.questIdentity(),
    empty: api.questIdentity({}),
    both: api.questIdentity({ id: 'death', name: 'Death Plateau' }),
    array: api.questIdentity(['death']),
    number: api.questIdentity({ id: 314 }),
    numberName: api.questIdentity({ name: 1 }),
    prereqOmitted: api.questPrereqs(),
    prereqEmpty: api.questPrereqs({}),
    prereqName: api.questPrereqs({ name: 'cook' }),
    prereqNumber: api.questPrereqs({ id: 219 }),
    blank: api.questIdentity({ id: '   ' }),
    blankPrereq: api.questPrereqs({ id: '   ' }),
    hit: api.questIdentity({ id: 'death' }),
    miss: api.questIdentity({ name: 'routequest' }),
    hitPrereq: api.questPrereqs({ id: 'cook' }),
  });
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    post_base(&iso, 1);
    iso.on_game_tick(1);
    let empty: serde_json::Value =
        serde_json::from_str(iso.probe("globalThis.__probe").unwrap().as_str().unwrap()).unwrap();
    iso.join();
    for key in [
        "omitted",
        "empty",
        "both",
        "array",
        "number",
        "numberName",
        "prereqOmitted",
        "prereqEmpty",
        "prereqName",
        "prereqNumber",
    ] {
        assert_eq!(empty[key]["error"], "invalid-args", "{key} {empty:?}");
    }
    for key in ["blank", "blankPrereq", "hit", "miss", "hitPrereq"] {
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
    assert_eq!(missing["number"]["error"], "invalid-args");
    assert_eq!(missing["prereqNumber"]["error"], "invalid-args");
    assert_eq!(missing["omitted"]["error"], "invalid-args");
    assert_eq!(
        missing["blank"]["error"],
        "family-unavailable:quest_identity"
    );
    assert_eq!(missing["hit"]["error"], "family-unavailable:quest_identity");
    assert_eq!(
        missing["miss"]["error"],
        "family-unavailable:quest_identity"
    );
    assert_eq!(
        missing["blankPrereq"]["error"],
        "family-unavailable:quest_prereqs"
    );
    assert_eq!(
        missing["hitPrereq"]["error"],
        "family-unavailable:quest_prereqs"
    );
}

#[test]
fn v2_install_is_typed_not_a_json_op_or_interact() {
    let value = probe(
        r#"
export const apiVersion = 2;
export function tick(api) {
  const row = api.questIdentity({ id: 'cook' });
  let requested = null;
  try { api.request({ op: 'questIdentity' }); requested = 'ok'; }
  catch (e) { requested = String(e && (e.message || e)); }
  globalThis.__probe = JSON.stringify({
    rowOk: row.ok === true,
    interact: globalThis.__rs2b0t_host && globalThis.__rs2b0t_host.interact,
    requested,
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
fn example_quest_facts_v2_is_one_read_only_fact_call() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("quest_facts_v2.ts");
    let src = std::fs::read_to_string(&path).expect("example source");
    assert!(!src.contains("request("));
    assert!(!src.contains("h.interact"));
    assert!(!src.contains("0xF800"));
    assert!(!src.contains("0x00F800"));
    assert!(!src.contains("%qp"));
    assert!(!src.contains("varp"));
    assert_eq!(src.matches("api.quest").count(), 1);
    let js = script::transpile_ts(&src).expect("transpile quest_facts_v2.ts");
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
        .find(|line| line.contains("\"id\""))
        .unwrap_or_else(|| panic!("example logged a fact call; logs={logs:?}"));
    let row: serde_json::Value = serde_json::from_str(last).unwrap();
    assert_eq!(row["ok"], true, "{row:?}");
    assert_eq!(row["value"]["id"], "death");
    assert_eq!(row["value"]["varp"], "death_equiproom");
    assert!(row["value"].get("rows").is_none(), "{row:?}");
}
