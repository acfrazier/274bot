use api::game_data::{self, SelectedGameData};
use api::quest_facts::QuestCatalog;
use api::selected::{ClientRevision, FactKey};
use script::quester::compile::{compile_uncached_for_test, CompileError};
use script::quester::path::PathDocument;
use script::quester::queue::ReleaseIndex;
use script::quester::schema::{path_schema_path, render};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn paths_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("paths/289")
}

fn selected_and_quests() -> (std::sync::Arc<SelectedGameData>, QuestCatalog) {
    let selected = game_data::for_revision(ClientRevision::R289).expect("289 selected data");
    let quests = QuestCatalog::from_identity(selected.quest_identity()).expect("289 quest catalog");
    (selected, quests)
}

fn compile_value(
    value: Value,
    selected: &SelectedGameData,
    quests: &QuestCatalog,
) -> Result<std::sync::Arc<script::quester::compile::CompiledPath>, CompileError> {
    let document: PathDocument = serde_json::from_value(value).expect("Path envelope decodes");
    compile_uncached_for_test(&document, selected, quests)
}

fn read_path(path: &Path) -> Value {
    let bytes =
        std::fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("decode {}: {error}", path.display()))
}

fn find_step_mut<'a>(value: &'a mut Value, id: &str) -> Option<&'a mut Value> {
    if value.as_object().is_some_and(|object| {
        object.get("kind").is_some() && object.get("id").and_then(Value::as_str) == Some(id)
    }) {
        return Some(value);
    }
    match value {
        Value::Array(array) => {
            for child in array {
                if let Some(step) = find_step_mut(child, id) {
                    return Some(step);
                }
            }
        }
        Value::Object(object) => {
            for child in object.values_mut() {
                if let Some(step) = find_step_mut(child, id) {
                    return Some(step);
                }
            }
        }
        _ => {}
    }
    None
}

fn expect_compile_error(
    value: Value,
    selected: &SelectedGameData,
    quests: &QuestCatalog,
) -> CompileError {
    match compile_value(value, selected, quests) {
        Err(error) => error,
        Ok(_) => panic!("Path unexpectedly compiled"),
    }
}

#[test]
fn path_schema_matches_generator() {
    let actual = std::fs::read_to_string(path_schema_path()).expect("read path.schema.json");
    assert_eq!(
        actual,
        render(),
        "{} is stale; run: cargo test -p script --test path_schema regen_path_schema -- --ignored",
        path_schema_path().display()
    );
}

/// Redberries come from the respawning Varrock ground spawns rather than
/// Wydin's one-stock shop: bank withdraw, then the patch walk, then Take.
fn assert_redberries_from_ground_spawns(steps: &[Value], qty: i64) {
    let bank_withdraw = steps
        .iter()
        .position(|step| {
            step["kind"] == "bank"
                && step["args"]["op"] == "withdraw"
                && step["args"]["items"][0]["obj"] == "redberries"
        })
        .expect("bank withdrawal before gathering");
    assert_eq!(steps[bank_withdraw]["args"]["items"][0]["qty"], json!(qty));

    let patch_tile = json!([3271, 3366, 0]);
    let patch_walk = steps
        .iter()
        .position(|step| step["kind"] == "walk" && step["args"]["tile"] == patch_tile)
        .expect("walk to a redberry patch");
    assert_eq!(steps[patch_walk]["args"]["radius"], 3);

    let collect = steps
        .iter()
        .position(|step| {
            step["kind"] == "interact"
                && step["args"]["target"]["ground"] == "redberries"
                && step["args"]["op"] == "Take"
        })
        .expect("take respawning ground redberries");
    assert!(bank_withdraw < patch_walk && patch_walk < collect);
    assert_eq!(steps[collect]["args"]["anchor"]["tile"], patch_tile);
    assert_eq!(steps[collect]["args"]["radius"], 12);
    assert_eq!(steps[collect]["args"]["wait_if_missing"], true);
    assert_eq!(
        steps[collect]["args"]["until"],
        json!({ "obj": "redberries", "qty": qty })
    );
    assert_eq!(steps[collect]["args"]["settle_ms"], 300000);
    assert!(
        steps
            .iter()
            .all(|step| step["kind"] != "buy" || step["args"]["obj"] != "redberries"),
        "redberries must come from ground spawns rather than Wydin's one-stock shop"
    );
}

#[test]
fn gobdip_redberries_recipe_collects_three_ground_spawns() {
    let value = read_path(&paths_dir().join("gobdip.json"));
    let steps = value["quest"]["acquire"]["acquire:redberries"]
        .as_array()
        .expect("Gobdip redberry recipe");
    assert!(steps.iter().all(|step| step["kind"] != "buy"));
    assert_redberries_from_ground_spawns(steps, 3);
}

/// Redberry pie burns about half the time at Cooking 10 and the pie acquire
/// restarts its recipe after a burn (8 times by default), so its berries need
/// the respawning source too.
#[test]
fn squire_pie_recipe_takes_redberries_from_ground_spawns() {
    let value = read_path(&paths_dir().join("squire.json"));
    let steps = value["quest"]["acquire"]["acquire:pie"]
        .as_array()
        .expect("Squire pie recipe");
    assert_redberries_from_ground_spawns(steps, 1);
}

/// What a Knight's Sword pie pass (first or restarted) begins with: a burnt
/// pie is emptied before anything else, a banked pie (the live cell's
/// `givebank redberry_pie 1`) replaces the bake, and a bank known to hold no
/// pie goes straight to the bake.
#[test]
fn squire_pie_pass_empties_a_burnt_pie_then_prefers_a_banked_one() {
    use api::bank_memory::{BankMemory, Origin};
    use script::quester::probe::{known_empty_bank, Choice, Probe};

    let _home = script::IsolatedEnv::enter("path-schema-squire-pie");
    let (selected, quests) = selected_and_quests();
    let _gathering = script::quester::compile::prepare_for_test({
        let selected = Arc::clone(&selected);
        move |worker| selected.prepare_gathering(worker)
    })
    .expect("289 gather catalog");
    let document: PathDocument =
        serde_json::from_value(read_path(&paths_dir().join("squire.json")))
            .expect("squire decodes");
    let compiled =
        compile_uncached_for_test(&document, &selected, &quests).expect("squire compiles");
    let pie = selected
        .item_by_alias("redberry_pie")
        .expect("redberry_pie")
        .id;
    let empty_bank = known_empty_bank();
    let pie_bank = BankMemory::seeded(&[(pie, 1)], Origin::Session);
    let banked = Probe {
        path: &compiled,
        selected: &selected,
        quests: &quests,
        progress: &[],
        bank: &pie_bank,
    };
    let unbanked = Probe {
        bank: &empty_bank,
        ..banked
    };
    let rimmington = |items: &[(&str, i32)]| snapshot_with_items(2969, 3210, &selected, items);
    let step = |id: &str| Choice::Step(FactKey::new(id));

    assert_eq!(
        banked.recipe_choice("acquire:pie", &rimmington(&[("burnt_pie", 1)])),
        step("empty-burnt-pie")
    );
    assert_eq!(
        banked.recipe_choice("acquire:pie", &rimmington(&[])),
        step("withdraw-redberry-pie")
    );
    assert_eq!(
        unbanked.recipe_choice("acquire:pie", &rimmington(&[])),
        step("take-pie-dish")
    );
    assert_eq!(
        unbanked.recipe_choice("acquire:pie", &rimmington(&[("redberry_pie", 1)])),
        Choice::Exhausted
    );
}

#[test]
fn generated_schema_uses_numeric_bounds_without_rust_formats() {
    fn assert_no_numeric_formats(value: &Value) {
        match value {
            Value::Array(items) => {
                for item in items {
                    assert_no_numeric_formats(item);
                }
            }
            Value::Object(object) => {
                let numeric = match object.get("type") {
                    Some(Value::String(kind)) => matches!(kind.as_str(), "integer" | "number"),
                    Some(Value::Array(kinds)) => kinds.iter().any(|kind| {
                        kind.as_str()
                            .is_some_and(|kind| matches!(kind, "integer" | "number"))
                    }),
                    _ => false,
                };
                assert!(
                    !numeric || !object.contains_key("format"),
                    "numeric schema contains a nonstandard format: {object:?}"
                );
                for child in object.values() {
                    assert_no_numeric_formats(child);
                }
            }
            _ => {}
        }
    }

    let schema: Value = serde_json::from_str(&render()).expect("generated Path schema");
    assert_no_numeric_formats(&schema);

    let skill = &schema["$defs"]["SkillMinimum"]["properties"]["skill"];
    assert_eq!(skill["minimum"], json!(0));
    assert_eq!(skill["maximum"], json!(255));
    assert!(skill.get("format").is_none());

    let radius = &schema["$defs"]["WalkArgs"]["properties"]["radius"];
    assert_eq!(radius["minimum"], json!(0));
    assert_eq!(radius["maximum"], json!(65535));
    assert!(radius.get("format").is_none());
}
#[test]
#[ignore]
fn regen_path_schema() {
    std::fs::write(path_schema_path(), render()).expect("write path.schema.json");
}

#[test]
fn bundled_paths_decode_and_compile() {
    let _home = script::IsolatedEnv::enter("path-schema-bundled");
    let root = paths_dir();
    let index_path = root.join("index.json");
    let index: ReleaseIndex = serde_json::from_slice(
        &std::fs::read(&index_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", index_path.display())),
    )
    .unwrap_or_else(|error| panic!("decode {}: {error}", index_path.display()));
    assert_eq!(index.schema, 1, "the release index has its own schema");

    let (selected, quests) = selected_and_quests();
    let _gathering = script::quester::compile::prepare_for_test({
        let selected = Arc::clone(&selected);
        move |worker| selected.prepare_gathering(worker)
    })
    .expect("289 gather catalog");
    for entry in index.paths {
        // Unavailable rows may keep their authored Path validated here.
        let Some(file) = entry.file else {
            continue;
        };
        let path = root.join(&file);
        let bytes =
            std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let document: PathDocument = serde_json::from_slice(&bytes)
            .unwrap_or_else(|error| panic!("decode {}: {error}", path.display()));
        assert_eq!(document.id.0.as_ref(), entry.id, "{file} id mismatch");
        compile_uncached_for_test(&document, &selected, &quests).unwrap_or_else(|error| {
            panic!(
                "{file} path={} step={:?} code={} detail={:?}",
                error.path.0, error.step, error.code, error.detail
            )
        });
    }
    let fixture_dir = root.join("fixtures");
    let mut fixtures = std::fs::read_dir(&fixture_dir)
        .unwrap_or_else(|error| panic!("read {}: {error}", fixture_dir.display()))
        .map(|entry| entry.expect("read Path fixture directory entry").path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    fixtures.sort();
    assert!(!fixtures.is_empty(), "expected combat Path fixtures");
    for path in fixtures {
        let bytes =
            std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let document: PathDocument = serde_json::from_slice(&bytes)
            .unwrap_or_else(|error| panic!("decode {}: {error}", path.display()));
        compile_uncached_for_test(&document, &selected, &quests).unwrap_or_else(|error| {
            panic!(
                "{} path={} step={:?} code={} detail={:?}",
                path.display(),
                error.path.0,
                error.step,
                error.code,
                error.detail
            )
        });
    }
}

#[test]
fn unknown_talk_arguments_are_rejected_with_step_context() {
    let _home = script::IsolatedEnv::enter("path-schema-talk-errors");
    let root = paths_dir();
    let (selected, quests) = selected_and_quests();

    let mut value = read_path(&root.join("cook.json"));
    let step = find_step_mut(&mut value, "start").expect("Cook start step");
    step["args"]["radius"] = json!(1);
    let error = expect_compile_error(value, &selected, &quests);
    assert_eq!(error.path, FactKey::new("cook"));
    assert_eq!(error.step, Some(FactKey::new("start")));
    assert_eq!(error.code.as_ref(), "invalid-args");
    assert!(error
        .detail
        .as_deref()
        .is_some_and(|detail| detail.contains("radius")));

    let mut value = read_path(&root.join("cook.json"));
    let step = find_step_mut(&mut value, "start").expect("Cook start step");
    step["args"]["anchor"]["radius"] = json!(1);
    let error = expect_compile_error(value, &selected, &quests);
    assert_eq!(error.path, FactKey::new("cook"));
    assert_eq!(error.step, Some(FactKey::new("start")));
    assert_eq!(error.code.as_ref(), "invalid-args");
    assert!(error
        .detail
        .as_deref()
        .is_some_and(|detail| detail.contains("radius")));

    let mut value = read_path(&root.join("sheep.json"));
    let step = find_step_mut(&mut value, "shear").expect("Sheep shear step");
    step["args"]["until"]["qty"]["radius"] = json!(1);
    let error = expect_compile_error(value, &selected, &quests);
    assert_eq!(error.path, FactKey::new("sheep"));
    assert_eq!(error.step, Some(FactKey::new("shear")));
    assert_eq!(error.code.as_ref(), "invalid-args");
    assert!(error
        .detail
        .as_deref()
        .is_some_and(|detail| detail.contains("radius")));
}

#[test]
fn invalid_args_keep_step_context_in_recipes_and_prelude() {
    let _home = script::IsolatedEnv::enter("path-schema-nested-errors");
    let root = paths_dir();
    let (selected, quests) = selected_and_quests();

    let mut value = read_path(&root.join("cook.json"));
    let step = find_step_mut(&mut value, "take-egg").expect("Cook recipe step");
    step["args"]["unknown"] = json!(true);
    let error = expect_compile_error(value, &selected, &quests);
    assert_eq!(error.path, FactKey::new("cook"));
    assert_eq!(error.step, Some(FactKey::new("take-egg")));
    assert_eq!(error.code.as_ref(), "invalid-args");
    assert!(error
        .detail
        .as_deref()
        .is_some_and(|detail| detail.contains("unknown")));

    let mut value = read_path(&root.join("cook.json"));
    let mut prelude_step = find_step_mut(&mut value, "start")
        .expect("Cook start step")
        .clone();
    prelude_step["id"] = json!("prelude-bad");
    prelude_step["args"]["unknown"] = json!(true);
    value["roles"][0]["prelude"]
        .as_array_mut()
        .expect("prelude array")
        .push(prelude_step);
    let error = expect_compile_error(value, &selected, &quests);
    assert_eq!(error.path, FactKey::new("cook"));
    assert_eq!(error.step, Some(FactKey::new("prelude-bad")));
    assert_eq!(error.code.as_ref(), "invalid-args");
    assert!(error
        .detail
        .as_deref()
        .is_some_and(|detail| detail.contains("unknown")));
}

#[test]
fn advances_checks_cover_historical_and_dynamic_quantity_cases() {
    let _home = script::IsolatedEnv::enter("path-schema-advances-checks");
    let root = paths_dir();
    let (selected, quests) = selected_and_quests();

    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/paths/runemysteries-ec8f29ad1.json");
    let error = expect_compile_error(read_path(&fixture), &selected, &quests);
    assert_eq!(error.path, FactKey::new("runemysteries"));
    assert_eq!(error.step, Some(FactKey::new("deliver-package")));
    assert_eq!(error.code.as_ref(), "advances-undeclared");

    let mut value = read_path(&root.join("cook.json"));
    let step = find_step_mut(&mut value, "hand-in").expect("Cook hand-in step");
    step["advances"] = json!(false);
    let settle = step["settle"].clone();
    step["settle"] = json!({ "All": [{ "Any": [{ "Not": settle }] }] });
    let error = expect_compile_error(value, &selected, &quests);
    assert_eq!(error.path, FactKey::new("cook"));
    assert_eq!(error.step, Some(FactKey::new("hand-in")));
    assert_eq!(error.code.as_ref(), "settle-needs-advance");
    assert!(error
        .detail
        .as_deref()
        .is_some_and(|detail| detail.contains("quest_colour")));

    let mut sheep_value = read_path(&root.join("sheep.json"));
    for id in ["shear", "spin"] {
        let step = find_step_mut(&mut sheep_value, id).expect("Sheep step");
        assert_eq!(step["advances"], json!(false));
        assert_eq!(step["settle"]["Fact"]["kind"], json!("item_count_at_least"));
        assert!(step["settle"]["Fact"]["args"]["qty"]["progress"].is_object());
    }
    compile_value(sheep_value, &selected, &quests)
        .expect("Sheep dynamic quantity settles are not progress facts");
}

#[test]
fn default_advance_class_defaults_false_and_accepts_true() {
    let _home = script::IsolatedEnv::enter("path-schema-default-advances");
    let root = paths_dir();
    let (selected, quests) = selected_and_quests();
    let mut value = read_path(&root.join("cook.json"));
    let step = json!({
        "id": "schema-default-walk",
        "kind": "walk",
        "version": 1,
        "args": { "tile": [3209, 3215, 0], "source": "PATH-SCHEMA-1 test" },
        "skip_if": { "Any": [] },
        "settle": { "Any": [] }
    });
    value["roles"][0]["prelude"] = json!([step]);
    let compiled = compile_value(value.clone(), &selected, &quests).expect("Default walk compiles");
    assert!(!compiled.prelude[0].advances);

    find_step_mut(&mut value, "schema-default-walk").expect("test walk step")["advances"] =
        json!(true);
    let compiled = compile_value(value, &selected, &quests)
        .expect("explicit true is accepted for Default kind");
    assert!(compiled.prelude[0].advances);
}

#[test]
fn schema_3_envelope_corrections_are_typed() {
    let _home = script::IsolatedEnv::enter("path-schema-envelope");
    let root = paths_dir();
    let (selected, quests) = selected_and_quests();

    let mut value = read_path(&root.join("cook.json"));
    value["quest"]["bank"] = json!("anywhere");
    assert!(serde_json::from_value::<PathDocument>(value).is_err());

    let mut value = read_path(&root.join("cook.json"));
    let step = find_step_mut(&mut value, "start").expect("Cook start step");
    step["skip_if"] = json!({ "Fact": { "kind": "prayer_points_at_least", "version": 1, "args": { "level": 43 } } });
    compile_value(value.clone(), &selected, &quests).expect("prayer-point fact uses a level only");
    find_step_mut(&mut value, "start").expect("Cook start step")["skip_if"] = json!({
        "Fact": { "kind": "prayer_points_at_least", "version": 1, "args": { "skill": "prayer", "level": 43 } }
    });
    let error = expect_compile_error(value, &selected, &quests);
    assert_eq!(error.code.as_ref(), "invalid-args");
    assert!(error
        .detail
        .as_deref()
        .is_some_and(|detail| detail.contains("skill")));

    let mut value = read_path(&root.join("cook.json"));
    find_step_mut(&mut value, "start").expect("Cook start step")["skip_if"] = json!({
        "Fact": { "kind": "near", "version": 1, "args": { "tile": [20000, 3215, 0], "radius": 2 } }
    });
    let error = expect_compile_error(value, &selected, &quests);
    assert_eq!(error.code.as_ref(), "invalid-tile");

    let mut value = read_path(&root.join("cook.json"));
    find_step_mut(&mut value, "start").expect("Cook start step")["settle"] = json!({
        "Fact": { "kind": "quest_colour", "version": 1, "args": { "quest": "cook", "is": "done" } }
    });
    let error = expect_compile_error(value, &selected, &quests);
    assert_eq!(error.code.as_ref(), "invalid-args");
    assert!(error
        .detail
        .as_deref()
        .is_some_and(|detail| detail.contains("done")));
}

#[test]
fn recursive_wait_predicate_inside_recipe_compiles() {
    let _home = script::IsolatedEnv::enter("path-schema-recursive-wait");
    let root = paths_dir();
    let (selected, quests) = selected_and_quests();
    let mut value = read_path(&root.join("cook.json"));
    let wait = json!({
        "id": "wait-for-egg",
        "kind": "wait",
        "version": 1,
        "args": {
            "until": {
                "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "egg" } }
            },
            "max_ticks": 1
        },
        "skip_if": { "Any": [] },
        "settle": { "Any": [] }
    });
    value["quest"]["acquire"]["acquire:egg"]
        .as_array_mut()
        .expect("Cook egg recipe")
        .insert(0, wait);
    compile_value(value, &selected, &quests)
        .expect("recursive wait predicate is valid inside an acquire recipe");
}

#[test]
fn symbolic_skill_requirement_uses_stats_index() {
    let _home = script::IsolatedEnv::enter("path-schema-symbolic-skill");
    let root = paths_dir();
    let (selected, quests) = selected_and_quests();
    let mut value = read_path(&root.join("cook.json"));
    value["quest"]["requirements"] = json!([{
        "id": "agility",
        "kind": { "Skill": { "skill": "agility", "level": 25 } },
        "at": "Start",
        "source": "schema-3 fixture"
    }]);

    let compiled =
        compile_value(value, &selected, &quests).expect("symbolic skill requirement compiles");
    let requirement = &compiled.eligibility.requirements[0];
    assert_eq!(requirement.id, FactKey::new("agility"));
    assert!(requirement.at_start);
    assert_eq!(requirement.source.as_ref(), "schema-3 fixture");
    assert!(matches!(
        &requirement.kind,
        api::selected::RequirementKind::Skill(minimum)
            if minimum.skill == 16 && minimum.level == 25
    ));
}

fn bundled_documents() -> Vec<(String, PathDocument)> {
    let root = paths_dir();
    let index: ReleaseIndex =
        serde_json::from_slice(&std::fs::read(root.join("index.json")).expect("read index.json"))
            .expect("decode index.json");
    index
        .paths
        .into_iter()
        .filter_map(|entry| entry.file)
        .map(|file| {
            let document: PathDocument = serde_json::from_value(read_path(&root.join(&file)))
                .unwrap_or_else(|error| panic!("decode {file}: {error}"));
            (file, document)
        })
        .collect()
}

/// First-win journal resolution makes a later rule unreachable when an earlier
/// rule's needles are contained in its text (the Plague City 24/25 shape). Every
/// bundled rule, fed exactly its own needles, must resolve to itself. Gathering
/// Paths (Monk's Friend) compile against the shared catalog, which only lives
/// while someone holds it, so this test holds its own like its siblings do.
#[test]
fn bundled_paths_have_no_shadowed_journal_rules() {
    let _home = script::IsolatedEnv::enter("path-schema-shadows");
    let (selected, quests) = selected_and_quests();
    let _gathering = script::quester::compile::prepare_for_test({
        let selected = Arc::clone(&selected);
        move |worker| selected.prepare_gathering(worker)
    })
    .expect("289 gather catalog");
    for (file, document) in bundled_documents() {
        let compiled = compile_uncached_for_test(&document, &selected, &quests)
            .unwrap_or_else(|error| panic!("{file}: {}", error.code));
        script::quester::probe::assert_no_shadowed_rules(&compiled);
    }
}

/// `Probe::choice` answers "which step would start here?" for a seeded
/// snapshot, so a Path's resume points can be pinned without a live run.
#[test]
fn selection_probe_follows_cook_inventory_and_ordered_cursor() {
    use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer, ItemView};
    use script::quester::path::SequenceOrder;
    use script::quester::probe::{known_empty_bank, Choice, Probe};

    let _home = script::IsolatedEnv::enter("path-schema-probe");
    let (selected, quests) = selected_and_quests();
    let mut document: PathDocument =
        serde_json::from_value(read_path(&paths_dir().join("cook.json"))).expect("cook decodes");
    let bank = known_empty_bank();
    let mut empty = GameSnapshot::new();
    empty.seed_ingame(2);
    empty.seed_inventory(Vec::new(), 28);
    empty.seed_equipment(Vec::new());
    let egg = selected.item_by_alias("egg").expect("egg");
    let mut holding_egg = GameSnapshot::new();
    holding_egg.seed_ingame(2);
    holding_egg.seed_inventory(
        vec![ItemView {
            def: api::obj_names::ItemDefView {
                id: egg.id,
                name: Some("Egg".into()),
                stackable: false,
                members: false,
                base_value: 1,
                noted: false,
                certificate_link: -1,
                certificate_template: -1,
            },
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 0,
            count: 1,
            actions: Vec::new(),
            component_id: 0,
        }],
        28,
    );
    holding_egg.seed_equipment(Vec::new());

    let compiled = compile_uncached_for_test(&document, &selected, &quests).expect("cook compiles");
    let probe = Probe {
        path: &compiled,
        selected: &selected,
        quests: &quests,
        progress: &[],
        bank: &bank,
    };
    assert_eq!(
        probe.choice("cook:1", 0, &empty),
        Choice::Step(FactKey::new("egg"))
    );
    assert_eq!(
        probe.choice("cook:1", 0, &holding_egg),
        Choice::Step(FactKey::new("milk")),
        "a held egg skips its acquisition"
    );

    // The same sequence in ordered form resumes at the cursor instead.
    document.roles[0].sequences[1].order = SequenceOrder::Ordered;
    let ordered =
        compile_uncached_for_test(&document, &selected, &quests).expect("ordered cook compiles");
    let probe = Probe {
        path: &ordered,
        selected: &selected,
        quests: &quests,
        progress: &[],
        bank: &bank,
    };
    assert_eq!(
        probe.choice("cook:1", 2, &empty),
        Choice::Step(FactKey::new("flour"))
    );
    assert_eq!(probe.choice("cook:1", 4, &empty), Choice::Exhausted);
}
/// REVIEW-PATHS-MEMBERS-A items H6/H7: the `elena:25` rule only matched varp
/// 26+ journal text while rule `elena:26` precedes it, so the clerk step could
/// never settle and the Bravek steps never ran. The merge into `elena:24-25`
/// plus the corrected/new area boxes must keep resolving; this guards both
/// the first-win rule order and the box data. Journal lines mirror
/// `scripts/quests/quest_elena/scripts/elena_journal.rs2`.
#[test]
fn elena_stage_24_25_merge_and_area_boxes_resolve() {
    use script::quester::progress::normalize_journal;
    use std::sync::Arc;

    fn resolve(compiled: &script::quester::compile::CompiledPath, lines: &[&str]) -> String {
        let arcs: Vec<Arc<str>> = lines.iter().map(|line| Arc::<str>::from(*line)).collect();
        let text = normalize_journal(&arcs);
        compiled
            .progress
            .rules
            .iter()
            .find(|rule| {
                rule.all.iter().all(|needle| text.contains(needle.as_ref()))
                    && (rule.any.is_empty()
                        || rule.any.iter().any(|needle| text.contains(needle.as_ref())))
                    && rule
                        .not
                        .iter()
                        .all(|needle| !text.contains(needle.as_ref()))
            })
            .map(|rule| rule.stage.0.as_ref().to_string())
            .unwrap_or_else(|| "journal-no-match".to_string())
    }

    let _home = script::IsolatedEnv::enter("path-schema-elena-24-25");
    let root = paths_dir();
    let (selected, quests) = selected_and_quests();

    let elena_value = read_path(&root.join("elena.json"));
    let elena = compile_value(elena_value.clone(), &selected, &quests).expect("elena compiles");

    // Every sequence stage resolves to an authored rule: no dead stages.
    for sequence in &elena.sequences {
        let stage = sequence.stage.0.as_ref();
        assert!(
            elena.progress.rule_for_stage(&sequence.stage).is_some(),
            "sequence {stage} has no progress rule"
        );
    }
    let rule_stages: Vec<&str> = elena
        .progress
        .rules
        .iter()
        .map(|rule| rule.stage.0.as_ref())
        .collect();
    assert!(rule_stages.contains(&"elena:24-25"), "merged rule missing");
    assert!(
        !rule_stages.contains(&"elena:25"),
        "dead rule elena:25 still present"
    );
    assert!(
        !rule_stages.contains(&"elena:24"),
        "unmerged rule elena:24 still present"
    );

    // Varp spoke_to_plague_house / spoke_to_clerk share one journal paragraph,
    // so both resolve to the merged stage even though older struck lines also match.
    let clerk_lines = [
        "I've spoken to Jethick, he thinks Elena was staying with",
        "the Rehnison Family, in a timber house to the north of the city.",
        "I've spoken to Milli about Elena, she says Elena was taken",
        "into one of the Plague Houses.",
        "I need clearance from either the Head Mourner or Bravek",
        "to get into the Plague House",
    ];
    assert_eq!(resolve(&elena, &clerk_lines), "elena:24-25");
    // Varp spoke_to_bravek adds the cure paragraph, which rule 26 (ordered
    // before 24-25) claims.
    let bravek_lines = [
        "I've spoken to Milli about Elena, she says Elena was taken",
        "into one of the Plague Houses.",
        "I need clearance from either the Head Mourner or Bravek",
        "to get into the Plague House",
        "Bravek might give me clearance if I make his Hangover",
        "Cure.",
        "I need to bring Bravek the Hangover Cure",
        "when I work it out.",
    ];
    assert_eq!(resolve(&elena, &bravek_lines), "elena:26");

    // H7 plus the new sewer box the picture/manhole recoveries navigate by.
    assert_eq!(
        elena_value["quest"]["areas"]["elena_house_interior"]["boxes"],
        json!([[2533, 3264, 2544, 3271, 0]])
    );
    assert_eq!(
        elena_value["quest"]["areas"]["elena_sewer"]["boxes"],
        json!([[2528, 9696, 2570, 9740, 0]])
    );

    // The merged sequence runs ordered now: the clerk, door and Bravek steps
    // leave no observable done evidence (varp 24 vs 25 share journal text),
    // so the cursor runs each once instead of re-selecting the clerk.
    // Behaviour is pinned by ordered_plague_city_clerk_door_bravek_runs_once_each.
    let merged = elena_value["roles"][0]["sequences"]
        .as_array()
        .expect("sequences")
        .iter()
        .find(|sequence| sequence["stage"] == json!("elena:24-25"))
        .expect("merged sequence");
    assert_eq!(merged["order"], json!("ordered"));

    // Hazeel cave box the entry-raft recovery settles in.
    let hazeel_value = read_path(&root.join("hazeelcult.json"));
    let hazeel =
        compile_value(hazeel_value.clone(), &selected, &quests).expect("hazeelcult compiles");
    for sequence in &hazeel.sequences {
        let stage = sequence.stage.0.as_ref();
        assert!(
            hazeel.progress.rule_for_stage(&sequence.stage).is_some(),
            "sequence {stage} has no progress rule"
        );
    }
    assert_eq!(
        hazeel_value["quest"]["areas"]["hazeel_cave"]["boxes"],
        json!([[2560, 9672, 2576, 9690, 0]])
    );
}

/// Empty-inventory snapshot with the local player at `(x, z)`: `in_area` and
/// `near` facts resolve off the player tile, and an empty scene keeps every
/// `loc_present` false.
fn snapshot_at(x: i32, z: i32) -> api::snapshot::GameSnapshot {
    snapshot_at_level(x, z, 0)
}

/// [`snapshot_at`] on map level `level`, for areas authored above the ground.
fn snapshot_at_level(x: i32, z: i32, level: i32) -> api::snapshot::GameSnapshot {
    let mut snapshot = api::snapshot::GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_local_player(api::snapshot::LocalPlayerView {
        player: api::snapshot::PlayerView {
            index: 0,
            network: api::WorldTile { x, z, level },
            actor: api::snapshot::ActorView {
                name: None,
                actions: vec![],
                tile: api::WorldTile { x, z, level },
                distance: 0,
                animation: -1,
                animation_frame: 0,
                pose_animation: -1,
                orientation: 0,
                target_orientation: 0,
                overhead_text: None,
                spot_animation: -1,
                spot_animation_stamp: 0,
                health: 10,
                total_health: 10,
                face_entity: -1,
                target: None,
                moving: false,
                running: false,
                in_combat: false,
            },
            combat_level: 3,
            skill_level: 0,
            headicons: 0,
            weapon: None,
        },
        energy: 100,
        weight: 0,
    });
    snapshot.seed_inventory(Vec::new(), 28);
    snapshot.seed_equipment(Vec::new());
    snapshot
}

/// REVIEW-PATHS-MEMBERS-A-R2 F1: the five sewer valves leave no observable
/// state, so authored order re-selected valve 1 forever and the cave entry
/// bounced back up the stairs. The `hazeelcult:4` sequence runs ordered now;
/// each cursor resumes at its own step.
#[test]
fn ordered_hazeel_valves_run_once_each_and_reach_the_fight() {
    use script::quester::probe::{known_empty_bank, progress_for_stage, Choice, Probe};

    let _home = script::IsolatedEnv::enter("path-schema-hazeel-valves");
    let (selected, quests) = selected_and_quests();
    let document: PathDocument =
        serde_json::from_value(read_path(&paths_dir().join("hazeelcult.json")))
            .expect("hazeelcult decodes");
    let compiled =
        compile_uncached_for_test(&document, &selected, &quests).expect("hazeelcult compiles");
    let bank = known_empty_bank();
    let progress = [progress_for_stage(
        &compiled,
        &selected,
        "hazeelcult:4",
        &[],
    )];
    let probe = Probe {
        path: &compiled,
        selected: &selected,
        quests: &quests,
        progress: &progress,
        bank: &bank,
    };

    // At the first valve with a fresh cursor the climb is skipped (surface)
    // and valve 1 starts.
    let at_valve_1 = snapshot_at(2562, 3247);
    assert_eq!(
        probe.choice("hazeelcult:4", 0, &at_valve_1),
        Choice::Step(FactKey::new("turn-sewervalve-1"))
    );
    // The cursor never re-selects valve 1: it resumes at its own valve.
    assert_eq!(
        probe.choice("hazeelcult:4", 2, &at_valve_1),
        Choice::Step(FactKey::new("turn-sewervalve-2"))
    );
    assert_eq!(
        probe.choice("hazeelcult:4", 5, &at_valve_1),
        Choice::Step(FactKey::new("turn-sewervalve-5"))
    );
    // Past the valves the bank steps skip (known-empty bank, nothing held)
    // and the cave entry starts.
    assert_eq!(
        probe.choice("hazeelcult:4", 6, &at_valve_1),
        Choice::Step(FactKey::new("enter-hazeel-cave"))
    );
    // In the cave the entry is skipped and the raft boards: no climb back up
    // once the cursor is past the valves.
    let in_cave = snapshot_at(2570, 9682);
    assert_eq!(
        probe.choice("hazeelcult:4", 9, &in_cave),
        Choice::Step(FactKey::new("board-raft-to-hideout"))
    );
    // REVIEW-PATHS-MEMBERS-A-R3 F-A: a restart replays from cursor 0 in every
    // location. On the surface the climb is skipped and the idempotent valves
    // re-run; at the surface cave entrance that means valve 1, not re-entry.
    assert_eq!(
        probe.choice("hazeelcult:4", 0, &snapshot_at(2585, 3234)),
        Choice::Step(FactKey::new("turn-sewervalve-1"))
    );
    // The Clivet handoff lands in the cave at cursor 0: climb up first, then
    // the valves re-run west to east before the cave is re-entered. There is
    // no cave-to-surface route, so skipping the climb here strands the bot.
    assert_eq!(
        probe.choice("hazeelcult:4", 0, &in_cave),
        Choice::Step(FactKey::new("climb-to-sewer-valves"))
    );
    // A hideout resume skips everything but the fight.
    let in_hideout = snapshot_at(2609, 9669);
    assert_eq!(
        probe.choice("hazeelcult:4", 0, &in_hideout),
        Choice::Step(FactKey::new("fight-alomone"))
    );
}

/// Snapshot at `(x, z)` holding one inventory slot per named item unit:
/// each `(alias, units)` pair seeds `units` slots of count 1 (buckets are
/// unstacked), resolved through the selected data so `has_item` and
/// `item_count_at_least` answer exactly as live.
fn snapshot_with_items(
    x: i32,
    z: i32,
    selected: &SelectedGameData,
    items: &[(&str, i32)],
) -> api::snapshot::GameSnapshot {
    snapshot_with_items_at_level(x, z, 0, selected, items)
}

/// [`snapshot_with_items`] on map level `level`.
fn snapshot_with_items_at_level(
    x: i32,
    z: i32,
    level: i32,
    selected: &SelectedGameData,
    items: &[(&str, i32)],
) -> api::snapshot::GameSnapshot {
    use api::snapshot::{ItemActionFamily, ItemContainer, ItemView};
    let mut snapshot = snapshot_at_level(x, z, level);
    let mut slot = 0;
    let mut rows = Vec::new();
    for &(alias, units) in items {
        let item = selected
            .item_by_alias(alias)
            .unwrap_or_else(|| panic!("unknown item alias {alias}"));
        for _ in 0..units {
            rows.push(ItemView {
                def: api::obj_names::ItemDefView {
                    id: item.id,
                    name: Some(alias.into()),
                    stackable: false,
                    members: false,
                    base_value: 1,
                    noted: false,
                    certificate_link: -1,
                    certificate_template: -1,
                },
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot,
                count: 1,
                actions: Vec::new(),
                component_id: 0,
            });
            slot += 1;
        }
    }
    snapshot.seed_inventory(rows, 28);
    snapshot
}

/// REVIEW-PATHS-MEMBERS-A-R3 F-C: the R3 cave-entry steps for stages 2 and 3
/// are authored (no cursor) and rely on skip/settle correctness. On the
/// surface the entry runs; underground it is skipped and the Clivet talk runs.
#[test]
fn hazeel_cave_entry_runs_on_surface_and_skips_underground() {
    use script::quester::probe::{known_empty_bank, progress_for_stage, Choice, Probe};

    let _home = script::IsolatedEnv::enter("path-schema-hazeel-cave-entry");
    let (selected, quests) = selected_and_quests();
    let document: PathDocument =
        serde_json::from_value(read_path(&paths_dir().join("hazeelcult.json")))
            .expect("hazeelcult decodes");
    let compiled =
        compile_uncached_for_test(&document, &selected, &quests).expect("hazeelcult compiles");
    let bank = known_empty_bank();
    let on_surface = snapshot_at(2565, 3271);
    let in_cave = snapshot_at(2570, 9682);

    let progress = [progress_for_stage(
        &compiled,
        &selected,
        "hazeelcult:2",
        &[],
    )];
    let probe = Probe {
        path: &compiled,
        selected: &selected,
        quests: &quests,
        progress: &progress,
        bank: &bank,
    };
    assert_eq!(
        probe.choice("hazeelcult:2", 0, &on_surface),
        Choice::Step(FactKey::new("enter-cave-to-meet-clivet"))
    );
    assert_eq!(
        probe.choice("hazeelcult:2", 0, &in_cave),
        Choice::Step(FactKey::new("meet-clivet"))
    );

    let progress = [progress_for_stage(
        &compiled,
        &selected,
        "hazeelcult:3",
        &[],
    )];
    let probe = Probe {
        path: &compiled,
        selected: &selected,
        quests: &quests,
        progress: &progress,
        bank: &bank,
    };
    assert_eq!(
        probe.choice("hazeelcult:3", 0, &on_surface),
        Choice::Step(FactKey::new("enter-cave-to-refuse-clivet"))
    );
    assert_eq!(
        probe.choice("hazeelcult:3", 0, &in_cave),
        Choice::Step(FactKey::new("refuse-clivet"))
    );
}

/// REVIEW-PATHS-MEMBERS-A-R3 F-B/F-C: the mud-pour refill must survive an
/// interrupted well fill. Each pour deletes one water and mints one empty
/// (`mud_patch.rs2:38-46`); past varp 7 pours hit the no-consume default
/// (`mud_patch.rs2:35`), so the refill must never under-pour into it. Only
/// 3+ held empties prove pours already ran; 2 water + 2 empties with 0 pours
/// done refills (the take holds, the fill tops up to 4) instead of pouring.
#[test]
fn elena_mud_refill_fetches_on_interrupted_fill_and_pours_on_resume() {
    use script::quester::probe::{known_empty_bank, progress_for_stage, Choice, Probe};

    let _home = script::IsolatedEnv::enter("path-schema-elena-mud-refill");
    let (selected, quests) = selected_and_quests();
    let document: PathDocument =
        serde_json::from_value(read_path(&paths_dir().join("elena.json"))).expect("elena decodes");
    let compiled =
        compile_uncached_for_test(&document, &selected, &quests).expect("elena compiles");
    let bank = known_empty_bank();
    let progress = [progress_for_stage(&compiled, &selected, "elena:03-06", &[])];
    let probe = Probe {
        path: &compiled,
        selected: &selected,
        quests: &quests,
        progress: &progress,
        bank: &bank,
    };
    // The rope is held throughout so the buy step skips and the bucket
    // family is what selects. Garden tile: east surface, no area gates here.
    let garden = |items: &[(&str, i32)]| snapshot_with_items(2566, 3331, &selected, items);
    // Stray empties with no water deposit first.
    assert_eq!(
        probe.choice(
            "elena:03-06",
            0,
            &garden(&[("rope", 1), ("bucket_empty", 3)])
        ),
        Choice::Step(FactKey::new("deposit-stray-empty-buckets"))
    );
    // F-B: 2 water + 2 empties with 0 pours done is an interrupted well
    // fill, not a mid-pour resume: refill instead of pouring twice.
    assert_eq!(
        probe.choice(
            "elena:03-06",
            0,
            &garden(&[("rope", 1), ("bucket_water", 2), ("bucket_empty", 2)])
        ),
        Choice::Step(FactKey::new("fetch-mud-water"))
    );
    // A fresh run with nothing held fetches too.
    assert_eq!(
        probe.choice("elena:03-06", 0, &garden(&[("rope", 1)])),
        Choice::Step(FactKey::new("fetch-mud-water"))
    );
    // A proven resume after 3 pours (1 water + 3 empties) pours directly.
    assert_eq!(
        probe.choice(
            "elena:03-06",
            0,
            &garden(&[("rope", 1), ("bucket_water", 1), ("bucket_empty", 3)])
        ),
        Choice::Step(FactKey::new("pour-water-on-mud"))
    );
    // A full set of 4 water pours directly.
    assert_eq!(
        probe.choice(
            "elena:03-06",
            0,
            &garden(&[("rope", 1), ("bucket_water", 4)])
        ),
        Choice::Step(FactKey::new("pour-water-on-mud"))
    );
}

/// REVIEW-PATHS-MEMBERS-A-R2 F2: the clerk step settled on a stage that was
/// already true with an unobservable varp difference, so authored order
/// re-selected the clerk inside Bravek's office forever. The `elena:24-25`
/// sequence runs ordered now; each cursor resumes at its own step.
#[test]
fn ordered_plague_city_clerk_door_bravek_runs_once_each() {
    use script::quester::probe::{known_empty_bank, progress_for_stage, Choice, Probe};

    let _home = script::IsolatedEnv::enter("path-schema-elena-2425");
    let (selected, quests) = selected_and_quests();
    let document: PathDocument =
        serde_json::from_value(read_path(&paths_dir().join("elena.json"))).expect("elena decodes");
    let compiled =
        compile_uncached_for_test(&document, &selected, &quests).expect("elena compiles");
    let bank = known_empty_bank();
    let progress = [progress_for_stage(&compiled, &selected, "elena:24-25", &[])];
    let probe = Probe {
        path: &compiled,
        selected: &selected,
        quests: &quests,
        progress: &progress,
        bank: &bank,
    };

    // At the clerk the introduction starts, then the cursor moves to the door:
    // never the clerk again.
    let at_clerk = snapshot_at(2528, 3317);
    assert_eq!(
        probe.choice("elena:24-25", 0, &at_clerk),
        Choice::Step(FactKey::new("get-clerk-introduction-to-bravek"))
    );
    assert_eq!(
        probe.choice("elena:24-25", 1, &at_clerk),
        Choice::Step(FactKey::new("open-bravek-office-door"))
    );
    // Inside the office the door is skipped and Bravek is asked directly.
    let in_office = snapshot_at(2536, 3314);
    assert_eq!(
        probe.choice("elena:24-25", 1, &in_office),
        Choice::Step(FactKey::new("ask-bravek-for-cure-recipe"))
    );
    assert_eq!(
        probe.choice("elena:24-25", 2, &in_office),
        Choice::Step(FactKey::new("ask-bravek-for-cure-recipe"))
    );
    // A restart safely replays the harmless clerk talk from step 1.
    assert_eq!(
        probe.choice("elena:24-25", 0, &in_office),
        Choice::Step(FactKey::new("get-clerk-introduction-to-bravek"))
    );
}

/// REVIEW-PATHS-MEMBERS-A-R2 F3: the cellar-gate return skipped only on the
/// east surface or a gate loc in scene, so upstairs, west surface and sewer
/// all re-selected it and walked back to an unroutable gate. The basement
/// skip lets every other area advance down the return leg, while the basement
/// itself still opens the gate.
#[test]
fn plague_city_cellar_return_skips_the_gate_outside_the_basement() {
    use script::quester::probe::{known_empty_bank, progress_for_stage, Choice, Probe};

    let _home = script::IsolatedEnv::enter("path-schema-cellar-return");
    let (selected, quests) = selected_and_quests();
    let document: PathDocument =
        serde_json::from_value(read_path(&paths_dir().join("elena.json"))).expect("elena decodes");
    let compiled =
        compile_uncached_for_test(&document, &selected, &quests).expect("elena compiles");
    let bank = known_empty_bank();
    let progress = [progress_for_stage(&compiled, &selected, "elena:28", &[])];
    let probe = Probe {
        path: &compiled,
        selected: &selected,
        quests: &quests,
        progress: &progress,
        bank: &bank,
    };

    // Upstairs the gate is skipped and the house is left.
    assert_eq!(
        probe.choice("elena:28", 0, &snapshot_at(2536, 3271)),
        Choice::Step(FactKey::new("leave-plague-house-return"))
    );
    // West surface walks to the manhole.
    assert_eq!(
        probe.choice("elena:28", 0, &snapshot_at(2529, 3290)),
        Choice::Step(FactKey::new("walk-to-manhole-return"))
    );
    // Sewer climbs the mud pile out east.
    assert_eq!(
        probe.choice("elena:28", 0, &snapshot_at(2540, 9710)),
        Choice::Step(FactKey::new("climb-mud-pile-return"))
    );
    // East reports to Edmond.
    assert_eq!(
        probe.choice("elena:28", 0, &snapshot_at(2566, 3331)),
        Choice::Step(FactKey::new("report-rescue-to-edmond"))
    );
    // In the basement with no gate in scene the gate still opens.
    assert_eq!(
        probe.choice("elena:28", 0, &snapshot_at(2539, 9672)),
        Choice::Step(FactKey::new("open-cellar-gate-return"))
    );
}

/// REVIEW-PATHS-MEMBERS-A-R2 test coverage: every rule of the three
/// Members-A Paths, fed exactly its own needles, must resolve to its own
/// stage, or first-win resolution has a shadowed rule.
#[test]
fn members_a_paths_have_no_shadowed_journal_rules() {
    let _home = script::IsolatedEnv::enter("path-schema-members-a-shadows");
    let (selected, quests) = selected_and_quests();
    // Monk's Friend gathers a log, so its compile needs the gather catalog.
    let _gathering = script::quester::compile::prepare_for_test({
        let selected = Arc::clone(&selected);
        move |worker| selected.prepare_gathering(worker)
    })
    .expect("289 gather catalog");
    for file in ["drunkmonk.json", "hazeelcult.json", "elena.json"] {
        let document: PathDocument = serde_json::from_value(read_path(&paths_dir().join(file)))
            .unwrap_or_else(|error| panic!("decode {file}: {error}"));
        let compiled = compile_uncached_for_test(&document, &selected, &quests)
            .unwrap_or_else(|error| panic!("{file}: {}", error.code));
        script::quester::probe::assert_no_shadowed_rules(&compiled);
    }
}

/// Integration 7 swapped Monk's Friend's `GAP-drunkmonk-woodcutting` wait for
/// a woodcutting gather. Stage 50's agree talk settles on entry (the journal
/// already reads stage 50), so the stage is ordered: the talk runs once, the
/// cursor then reaches the chop, and a held log goes straight to the hand-in.
#[test]
fn ordered_monks_friend_wood_agrees_once_then_chops_and_hands_in() {
    use script::quester::probe::{known_empty_bank, progress_for_stage, Choice, Probe};

    let _home = script::IsolatedEnv::enter("path-schema-drunkmonk-wood");
    let (selected, quests) = selected_and_quests();
    let _gathering = script::quester::compile::prepare_for_test({
        let selected = Arc::clone(&selected);
        move |worker| selected.prepare_gathering(worker)
    })
    .expect("289 gather catalog");
    let document: PathDocument =
        serde_json::from_value(read_path(&paths_dir().join("drunkmonk.json")))
            .expect("drunkmonk decodes");
    let compiled =
        compile_uncached_for_test(&document, &selected, &quests).expect("drunkmonk compiles");
    let bank = known_empty_bank();
    let progress = [progress_for_stage(
        &compiled,
        &selected,
        "drunkmonk:50",
        &[],
    )];
    let probe = Probe {
        path: &compiled,
        selected: &selected,
        quests: &quests,
        progress: &progress,
        bank: &bank,
    };
    let near_cedric = |items: &[(&str, i32)]| snapshot_with_items(2614, 3257, &selected, items);

    // No log: agree first, then the cursor moves on to the chop.
    assert_eq!(
        probe.choice("drunkmonk:50", 0, &near_cedric(&[])),
        Choice::Step(FactKey::new("agree-to-help-cedric"))
    );
    assert_eq!(
        probe.choice("drunkmonk:50", 1, &near_cedric(&[])),
        Choice::Step(FactKey::new("chop-normal-tree-log"))
    );
    // A held log skips the talk and the chop from any cursor.
    for cursor in 0..=2 {
        assert_eq!(
            probe.choice("drunkmonk:50", cursor, &near_cedric(&[("logs", 1)])),
            Choice::Step(FactKey::new("hand-logs-to-cedric")),
            "cursor {cursor}"
        );
    }
    assert!(
        !compiled
            .sequences
            .iter()
            .flat_map(|sequence| sequence.steps.iter())
            .any(|step| step.id.0.starts_with("GAP-")),
        "no GAP wait remains in Monk's Friend"
    );
}

/// DIAG-CRYPT-GATE-NAV: nav has no edge through either Priest in Peril crypt
/// gate, and Gate 2's west stand (3431,9897) is outside `monuments` and north
/// of Gate 1's wall line, so Gate 1's Open approach from there targets the
/// unreachable north side. Leaving from there walks to Gate 1's routable
/// south doorstep first; the Open then runs from inside `monuments`.
#[test]
fn priestperil_leave_crypt_walks_to_gate_1_south_from_the_gate_2_west_stand() {
    use script::quester::probe::{known_empty_bank, Choice, Probe};

    let _home = script::IsolatedEnv::enter("path-schema-priestperil-gates");
    let (selected, quests) = selected_and_quests();
    let _gathering = script::quester::compile::prepare_for_test({
        let selected = Arc::clone(&selected);
        move |worker| selected.prepare_gathering(worker)
    })
    .expect("289 gather catalog");
    let document: PathDocument =
        serde_json::from_value(read_path(&paths_dir().join("priestperil.json")))
            .expect("priestperil decodes");
    let compiled =
        compile_uncached_for_test(&document, &selected, &quests).expect("priestperil compiles");
    let bank = known_empty_bank();
    let probe = Probe {
        path: &compiled,
        selected: &selected,
        quests: &quests,
        progress: &[],
        bank: &bank,
    };
    let step = |id: &str| Choice::Step(FactKey::new(id));
    let leave = |x, z| probe.recipe_choice("leave:crypt", &snapshot_at(x, z));

    // East of Gate 2 the westbound Open runs first; it lands on 3431,9897.
    assert_eq!(leave(3432, 9897), step("crypt-return-through-second-gate"));
    assert_eq!(leave(3431, 9897), step("crypt-return-to-first-gate-south"));
    assert_eq!(leave(3405, 9894), step("crypt-return-through-first-gate"));
    assert_eq!(leave(3420, 9890), step("crypt-return-through-first-gate"));
    // North of Gate 1 the gates are behind: walk on to the ladder.
    assert_eq!(leave(3405, 9897), step("crypt-exit-approach"));
}

/// DIAG-PRIEST-TEMPLE: after the bless chase the player stands on the
/// south-east walkway (3416,3486,2), east of the prison wall and below
/// `cell_inside`. `leave-cell-with-holy-water` has to stay selected there:
/// douse walks the coffin's west side, which only the opened door reaches.
/// Standing west of the wall (or already out) douse runs directly.
#[test]
fn priestperil_stage_6_leaves_the_cell_from_the_south_east_walkway_before_dousing() {
    use script::quester::probe::{known_empty_bank, progress_for_stage, Choice, Probe};

    let _home = script::IsolatedEnv::enter("path-schema-priestperil-walkway");
    let (selected, quests) = selected_and_quests();
    let _gathering = script::quester::compile::prepare_for_test({
        let selected = Arc::clone(&selected);
        move |worker| selected.prepare_gathering(worker)
    })
    .expect("289 gather catalog");
    let document: PathDocument =
        serde_json::from_value(read_path(&paths_dir().join("priestperil.json")))
            .expect("priestperil decodes");
    let compiled =
        compile_uncached_for_test(&document, &selected, &quests).expect("priestperil compiles");
    let bank = known_empty_bank();
    let progress = [progress_for_stage(
        &compiled,
        &selected,
        "priestperil:6",
        &[],
    )];
    let probe = Probe {
        path: &compiled,
        selected: &selected,
        quests: &quests,
        progress: &progress,
        bank: &bank,
    };
    let step = |id: &str| Choice::Step(FactKey::new(id));
    let holding_holy_water =
        |x, z| snapshot_with_items_at_level(x, z, 2, &selected, &[("bucket_blessedwater", 1)]);
    let chosen = |x, z| probe.choice("priestperil:6", 0, &holding_holy_water(x, z));

    // Inside the cell, east of the door: the stall tile and its neighbours.
    assert_eq!(chosen(3416, 3486), step("leave-cell-with-holy-water"));
    assert_eq!(chosen(3418, 3484), step("leave-cell-with-holy-water"));
    assert_eq!(chosen(3417, 3488), step("leave-cell-with-holy-water"));
    // West of the wall at the overnight pour stand: no door left to open.
    assert_eq!(chosen(3413, 3488), step("douse-vampire-coffin"));
    assert_eq!(chosen(3414, 3487), step("douse-vampire-coffin"));
}

/// PORT-S6-DRAFTS-FIX F1: the Varrock stand spawns are `giantrat1` (id 87),
/// not `giantrat` (id 86), so the rat hunt must target both configs.
/// Fails on `45aec8ec3` where the target is the single `"giantrat"`.
#[test]
fn druid_rat_hunt_targets_both_giantrat_configs() {
    let _home = script::IsolatedEnv::enter("path-schema-druid-rats");
    let (selected, quests) = selected_and_quests();
    let mut value = read_path(&paths_dir().join("druid.json"));
    let step = find_step_mut(&mut value, "hunt-rat-for-meat").expect("hunt step");
    let npc = step
        .pointer("/args/target/npc")
        .expect("hunt target npc")
        .clone();
    let names: Vec<String> = match npc {
        Value::String(one) => vec![one],
        Value::Array(many) => many
            .iter()
            .map(|name| name.as_str().expect("npc name").to_string())
            .collect(),
        other => panic!("unexpected npc arg shape: {other}"),
    };
    assert!(
        names.contains(&"giantrat".to_string()),
        "keeps the base giantrat config: {names:?}"
    );
    assert!(
        names.contains(&"giantrat1".to_string()),
        "covers the stand spawns (id 87): {names:?}"
    );
    compile_value(value, &selected, &quests).expect("druid compiles");
}

/// PORT-S6-DRAFTS-FIX F3: `pick-snake` must not advance the journal: with only
/// the unidentified herb held the content journal prints no matching line, so
/// an `advances: true` read parks the run. Fails on `45aec8ec3` (`true`).
#[test]
fn junglepotion_pick_snake_does_not_advance() {
    let _home = script::IsolatedEnv::enter("path-schema-jungle-snake");
    let (selected, quests) = selected_and_quests();
    let mut value = read_path(&paths_dir().join("junglepotion.json"));
    let step = find_step_mut(&mut value, "pick-snake").expect("pick-snake");
    assert_eq!(
        step.get("advances"),
        Some(&serde_json::json!(false)),
        "pick-snake must settle without a journal re-read"
    );
    compile_value(value, &selected, &quests).expect("junglepotion compiles");
}

/// PORT-S6-DRAFTS-FIX F6: handing Fluffs to Gertrude advances the quest varp,
/// so the step must re-read the journal instead of replaying stage 4.
/// Fails on `45aec8ec3` (`false`).
#[test]
fn fluffs_kitten_handoff_advances() {
    let _home = script::IsolatedEnv::enter("path-schema-fluffs-kitten");
    let (selected, quests) = selected_and_quests();
    let mut value = read_path(&paths_dir().join("fluffs.json"));
    let step = find_step_mut(&mut value, "give-fluffs-her-kitten").expect("kitten handoff");
    assert_eq!(
        step.get("advances"),
        Some(&serde_json::json!(true)),
        "give-fluffs-her-kitten changes progress and must advance"
    );
    compile_value(value, &selected, &quests).expect("fluffs compiles");
}

/// PORT-S6-DRAFTS-FIX F5: the Brimhaven <-> Ardougne boat costs 30 coins each
/// way in content (`customs_officer.rs2:80-82` out, `captain_barnaby.rs2:2`
/// via `karamja_sailor_dialogue` back), so the non-`owns_inventory` Path must
/// float at least the 60-coin round trip. Fails on `45aec8ec3` (`0`).
#[test]
fn totem_coin_float_covers_the_round_trip_fare() {
    let _home = script::IsolatedEnv::enter("path-schema-totem-fare");
    let (selected, quests) = selected_and_quests();
    let value = read_path(&paths_dir().join("totem.json"));
    let float = value
        .pointer("/quest/coin_float")
        .and_then(serde_json::Value::as_i64)
        .expect("quest.coin_float");
    assert!(
        float >= 60,
        "totem coin_float {float} must cover the 30+30 round trip"
    );
    compile_value(value, &selected, &quests).expect("totem compiles");
}

/// PORT-S6-DRAFTS-FIX F2 (real pack): the yard is enclosed and nav does not
/// model the broken fence as a transport, so the stage-2/3 approach walks
/// must end outside it at (3305,3492); the old (3305,3496) target is NoPath
/// from outside even with zones exempt.
#[test]
#[ignore = "requires the real 289 nav pack at /Volumes/dev-scratch/274bot-evidence/CORE-INTEGRATOR-7/nav/289/274bot.navpack"]
fn fluffs_yard_entry_walks_end_outside_the_fence() {
    use api::WorldTile;
    use nav::router::{find_with, FindOptions};
    use nav::world::NavWorld;
    use nav::zones::ZoneExempt;
    use nav::WorldState;
    use std::path::PathBuf;

    let _home = script::IsolatedEnv::enter("path-schema-fluffs-fence-pack");
    let mut value = read_path(&paths_dir().join("fluffs.json"));
    for id in ["walk-to-yard-entry", "walk-to-yard-entry-for-sardine"] {
        let step = find_step_mut(&mut value, id).expect("yard walk");
        let tile = step.pointer("/args/tile").expect("walk tile").clone();
        assert_eq!(
            tile,
            serde_json::json!([3305, 3492, 0]),
            "{id} ends outside"
        );
    }

    let pack = PathBuf::from(
        "/Volumes/dev-scratch/274bot-evidence/CORE-INTEGRATOR-7/nav/289/274bot.navpack",
    );
    let mut world = NavWorld::load_pack(&pack).expect("real 289 pack");
    let (_o, _w, _h, flags) = nav::pack::decode_flags_sidecar(
        &std::fs::read(pack.with_extension("navflags")).expect("raw flags"),
    )
    .expect("decode flags");
    world.collision.attach_flags(flags);
    let from = WorldTile {
        x: 3255,
        z: 3288,
        level: 0,
    };
    let outside = WorldTile {
        x: 3305,
        z: 3492,
        level: 0,
    };
    let inside = WorldTile {
        x: 3305,
        z: 3496,
        level: 0,
    };
    let state = WorldState::empty().with_map_members(true);
    let free_opts = FindOptions {
        zones: ZoneExempt::all(),
        ..FindOptions::default()
    };
    find_with(
        &world.collision,
        &world.graph,
        from,
        outside,
        FindOptions::default(),
        &state,
    )
    .unwrap_or_else(|error| panic!("cow -> outside fence must route: {error:?}"));
    assert!(
        find_with(
            &world.collision,
            &world.graph,
            from,
            inside,
            free_opts,
            &state
        )
        .is_err(),
        "the old yard-side target stays unreachable even exempt"
    );
}

/// PORT-S6-DRAFTS-FIX F4 (real pack): the dungeon and herb legs cross named
/// danger zones, so the grant-carrying walks must resolve and route at low
/// combat where the strict search refuses.
#[test]
#[ignore = "requires the real 289 nav pack at /Volumes/dev-scratch/274bot-evidence/CORE-INTEGRATOR-7/nav/289/274bot.navpack"]
fn danger_cross_grants_route_low_combat_legs() {
    use api::WorldTile;
    use nav::router::{find_with, FindOptions};
    use nav::world::NavWorld;
    use nav::zones::ZoneExempt;
    use nav::WorldState;
    use std::path::PathBuf;

    let _home = script::IsolatedEnv::enter("path-schema-cross-pack");
    let mut druid = read_path(&paths_dir().join("druid.json"));
    for id in ["walk-to-prison-door", "walk-to-dungeon-exit"] {
        let step = find_step_mut(&mut druid, id).expect("dungeon walk");
        let cross = step
            .pointer("/args/cross")
            .expect("cross list")
            .as_array()
            .expect("cross array");
        assert!(cross.len() <= 8, "{id} fits the 8-grant limit");
        for zone in [
            "skeleton_unarmed@2882,9826,0",
            "skeleton_unarmed@2885,9819,0",
            "skeleton_armed@2884,9836,0",
        ] {
            assert!(
                cross.iter().any(|entry| entry.as_str() == Some(zone)),
                "{id} grants {zone}"
            );
        }
    }
    let mut jungle = read_path(&paths_dir().join("junglepotion.json"));
    let expected: &[(&str, &[&str])] = &[
        (
            "walk-to-snake-weed",
            &[
                "hobgoblin_unarmed@2787,3013,0",
                "jungle_spider@2780,3029,0",
                "tribesman@2771,3014,0",
            ],
        ),
        (
            "walk-to-volencia-moss",
            &["snake@2836,3043,0", "snake@2847,3042,0"],
        ),
        (
            "walk-to-rogues-purse",
            &["jogre@2826,9518,0", "jogre@2848,9483,0"],
        ),
    ];
    for (id, zones) in expected {
        let step = find_step_mut(&mut jungle, id).expect("herb walk");
        let cross = step
            .pointer("/args/cross")
            .expect("cross list")
            .as_array()
            .expect("cross array");
        for zone in *zones {
            assert!(
                cross.iter().any(|entry| entry.as_str() == Some(*zone)),
                "{id} grants {zone}"
            );
        }
    }

    let pack = PathBuf::from(
        "/Volumes/dev-scratch/274bot-evidence/CORE-INTEGRATOR-7/nav/289/274bot.navpack",
    );
    let mut world = NavWorld::load_pack(&pack).expect("real 289 pack");
    let (_o, _w, _h, flags) = nav::pack::decode_flags_sidecar(
        &std::fs::read(pack.with_extension("navflags")).expect("raw flags"),
    )
    .expect("decode flags");
    world.collision.attach_flags(flags);
    let table = world.graph.zones.as_ref().expect("baked zones");
    let tile = |x, z| WorldTile { x, z, level: 0 };
    // (from, to, combat, cross-list): the strict search refuses each of the
    // zone-gated legs at low combat; the named grants restore them.
    let legs = &[
        (
            (2884, 9797),
            (2888, 9831),
            20,
            &[
                "skeleton_unarmed@2882,9826,0",
                "skeleton_unarmed@2885,9819,0",
                "skeleton_unarmed@2885,9823,0",
                "skeleton_unarmed@2886,9812,0",
                "skeleton_unarmed@2886,9816,0",
                "skeleton_unarmed@2887,9821,0",
                "skeleton_armed@2884,9836,0",
                "poisonspider@2876,9806,0",
            ] as &[&str],
        ),
        (
            (2809, 3086),
            (2761, 3015),
            3,
            &[
                "hobgoblin_unarmed@2787,3013,0",
                "hobgoblin_unarmed@2791,3013,0",
                "hobgoblin_unarmed@2794,3013,0",
                "jungle_spider@2780,3029,0",
                "jungle_spider@2780,3031,0",
                "tribesman@2771,3014,0",
                "tribesman@2773,3017,0",
                "tribesman@2777,3068,0",
            ] as &[&str],
        ),
        (
            (2830, 9521),
            (2850, 9477),
            3,
            &[
                "jogre@2826,9518,0",
                "jogre@2834,9499,0",
                "jogre@2834,9513,0",
                "jogre@2836,9522,0",
                "jogre@2838,9490,0",
                "jogre@2848,9483,0",
            ] as &[&str],
        ),
    ];
    for ((fx, fz), (tx, tz), cl, cross) in legs {
        let mut state = WorldState::empty().with_map_members(true);
        state.combat_level = Some(*cl);
        assert!(
            find_with(
                &world.collision,
                &world.graph,
                tile(*fx, *fz),
                tile(*tx, *tz),
                FindOptions::default(),
                &state,
            )
            .is_err(),
            "strict refuses ({fx},{fz}) -> ({tx},{tz}) at combat {cl}"
        );
        let keys: Vec<_> = cross
            .iter()
            .map(|name| table.resolve(name).expect("known zone"))
            .collect();
        let named_opts = FindOptions {
            zones: ZoneExempt::named(&keys).expect("named grants"),
            ..FindOptions::default()
        };
        find_with(
            &world.collision,
            &world.graph,
            tile(*fx, *fz),
            tile(*tx, *tz),
            named_opts,
            &state,
        )
        .unwrap_or_else(|error| {
            panic!("named grants route ({fx},{fz}) -> ({tx},{tz}) at {cl}: {error:?}")
        });
    }
}
