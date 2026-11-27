//! `x:upass:cross_grid`: the Underground Pass floor grid. It compiles only
//! with its handler; the base without it rejects the kind as unknown.

use api::quest_facts::QuestCatalog;
use api::selected::ClientRevision;
use script::quester::compile::{compile_uncached_for_test, decode_cook, CompileError};
use script::quester::path::PredicateDocument;
use serde_json::{json, Value};

const KIND: &str = "x:upass:cross_grid";
const SOURCE: &str = "C:scripts/quests/quest_upass/scripts/upass_grid.rs2:66";

fn compile_with(args: Value, advances: Option<bool>) -> Result<(), CompileError> {
    let _home = script::IsolatedEnv::enter("quester-upass-grid");
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = QuestCatalog::from_identity(selected.quest_identity()).unwrap();
    let mut document = decode_cook().unwrap();
    let step = &mut document.roles[0].sequences[0].steps[0];
    step.kind = KIND.into();
    step.advances = advances;
    step.settle = PredicateDocument::All(Vec::new());
    step.args = args;
    compile_uncached_for_test(&document, &selected, &quests).map(|_| ())
}

fn arrival(tile: [i32; 3]) -> Value {
    json!({"arrival": {"tile": tile, "source": SOURCE}})
}

#[test]
fn far_side_tiles_west_of_the_grid_compile() {
    if !api::test_support::game_data_available() {
        return;
    }
    // x 2466 and west, in the five lanes (z 9673..=9682) on level 0.
    for tile in [[2466, 9677, 0], [2460, 9673, 0], [2466, 9682, 0]] {
        compile_with(arrival(tile), Some(false))
            .unwrap_or_else(|error| panic!("{tile:?}: {}", error.code));
    }
}

#[test]
fn tiles_inside_east_of_or_off_the_grid_do_not_compile() {
    if !api::test_support::game_data_available() {
        return;
    }
    for tile in [
        [2467, 9677, 0],
        [2476, 9677, 0],
        [2478, 9677, 0],
        [2466, 9677, 1],
        [2466, 9672, 0],
        [2466, 9683, 0],
        [-1, 9677, 0],
        [2466, 9677, 4],
    ] {
        assert!(
            compile_with(arrival(tile), Some(false)).is_err(),
            "{tile:?} must not compile"
        );
    }
}

#[test]
fn bad_arguments_do_not_compile() {
    if !api::test_support::game_data_available() {
        return;
    }
    for args in [
        json!({"arrival": {"tile": [2466, 9677, 0], "source": "  "}}),
        json!({"arrival": {"tile": [2466, 9677, 0]}}),
        json!({"arrival": {"tile": [2466, 9677], "source": SOURCE}}),
        json!({"tile": [2466, 9677, 0]}),
        json!({}),
        json!({"arrival": {"tile": [2466, 9677, 0], "source": SOURCE}, "lanes": [3, 3, 3]}),
        json!({"arrival": {"tile": [2466, 9677, 0], "source": SOURCE, "radius": 1}}),
    ] {
        assert!(
            compile_with(args.clone(), Some(false)).is_err(),
            "{args} must not compile"
        );
    }
}

#[test]
fn the_step_is_explicit_and_must_declare_advances() {
    if !api::test_support::game_data_available() {
        return;
    }
    let error = compile_with(arrival([2466, 9677, 0]), None).expect_err("advances is required");
    assert_eq!(error.code.as_ref(), "advances-undeclared");
    compile_with(arrival([2466, 9677, 0]), Some(true)).expect("advances: true compiles");
}
