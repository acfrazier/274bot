//! `answer_count` and `if_button` Path steps. Both compile only with the
//! family; the pre-family base rejects each as an unknown handler.

use api::quest_facts::QuestCatalog;
use api::selected::ClientRevision;
use script::quester::compile::{compile_uncached_for_test, decode_cook, CompileError};
use script::quester::path::PredicateDocument;
use serde_json::{json, Value};

fn compile_with(kind: &str, args: Value, advances: Option<bool>) -> Result<(), CompileError> {
    let _home = script::IsolatedEnv::enter("quester-prompt-family");
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = QuestCatalog::from_identity(selected.quest_identity()).unwrap();
    let mut document = decode_cook().unwrap();
    let step = &mut document.roles[0].sequences[0].steps[0];
    step.kind = kind.into();
    step.advances = advances;
    step.settle = PredicateDocument::All(Vec::new());
    step.args = args;
    compile_uncached_for_test(&document, &selected, &quests).map(|_| ())
}

fn compile(kind: &str, args: Value) -> Result<(), CompileError> {
    compile_with(kind, args, Some(false))
}

const SOURCE: &str = "death_guard_equiproom.rs2:227-228 p_countdialog";

#[test]
fn answer_count_compiles_an_authored_value_with_a_bound_and_a_source() {
    if !api::test_support::game_data_available() {
        return;
    }
    for value in [0, 101, 1000] {
        compile(
            "answer_count",
            json!({"value": value, "max_ticks": 3, "source": SOURCE}),
        )
        .unwrap_or_else(|error| panic!("{value}: {}", error.code));
    }
}

#[test]
fn answer_count_rejects_bad_values_bounds_sources_and_unknown_fields() {
    if !api::test_support::game_data_available() {
        return;
    }
    for args in [
        json!({"value": -1, "max_ticks": 3, "source": SOURCE}),
        json!({"value": 1, "max_ticks": 0, "source": SOURCE}),
        json!({"value": 1, "max_ticks": 3, "source": "  "}),
        json!({"value": 1, "max_ticks": 3}),
        json!({"max_ticks": 3, "source": SOURCE}),
        json!({"value": 1, "source": SOURCE}),
        json!({"value": 1, "max_ticks": 3, "source": SOURCE, "component": 8421}),
        json!({"value": "101", "max_ticks": 3, "source": SOURCE}),
    ] {
        assert!(
            compile("answer_count", args.clone()).is_err(),
            "{args} must not compile"
        );
    }
}

#[test]
fn if_button_compiles_a_component_with_a_bound_and_a_source() {
    if !api::test_support::game_data_available() {
        return;
    }
    // `death_dice:com_28` and the pause button `death_dice:com_30`
    // (`pack/interface.pack:8422`, `:8424`).
    for component in [8421, 8423] {
        compile(
            "if_button",
            json!({
                "component": component,
                "max_ticks": 12,
                "source": "death_guard_equiproom.rs2:46-47 [if_button,death_dice:com_28]",
            }),
        )
        .unwrap_or_else(|error| panic!("{component}: {}", error.code));
    }
}

#[test]
fn if_button_rejects_bad_components_bounds_sources_and_unknown_fields() {
    if !api::test_support::game_data_available() {
        return;
    }
    for args in [
        json!({"component": 0, "max_ticks": 12, "source": SOURCE}),
        json!({"component": -5, "max_ticks": 12, "source": SOURCE}),
        json!({"component": 8421, "max_ticks": 0, "source": SOURCE}),
        json!({"component": 8421, "max_ticks": 12, "source": ""}),
        json!({"component": 8421, "max_ticks": 12}),
        json!({"max_ticks": 12, "source": SOURCE}),
        json!({"component": 8421, "max_ticks": 12, "source": SOURCE, "interface": 6675}),
    ] {
        assert!(
            compile("if_button", args.clone()).is_err(),
            "{args} must not compile"
        );
    }
}

#[test]
fn both_steps_are_explicit_kinds_that_must_declare_advances() {
    if !api::test_support::game_data_available() {
        return;
    }
    for (kind, args) in [
        (
            "answer_count",
            json!({"value": 101, "max_ticks": 3, "source": SOURCE}),
        ),
        (
            "if_button",
            json!({"component": 8421, "max_ticks": 12, "source": SOURCE}),
        ),
    ] {
        let error = compile_with(kind, args.clone(), None).expect_err("advances is required");
        assert_eq!(error.code.as_ref(), "advances-undeclared", "{kind}");
        compile_with(kind, args, Some(true))
            .unwrap_or_else(|error| panic!("{kind} with advances: {}", error.code));
    }
}
