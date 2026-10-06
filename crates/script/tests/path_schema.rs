use api::game_data::{self, SelectedGameData};
use api::quest_facts::QuestCatalog;
use api::selected::{ClientRevision, FactKey};
use script::quester::compile::{compile_uncached_for_test, CompileError};
use script::quester::path::PathDocument;
use script::quester::queue::ReleaseIndex;
use script::quester::schema::{path_schema_path, render};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

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
/// bundled rule, fed exactly its own needles, must resolve to itself.
#[test]
fn bundled_paths_have_no_shadowed_journal_rules() {
    let _home = script::IsolatedEnv::enter("path-schema-shadows");
    let (selected, quests) = selected_and_quests();
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
