//! Authored Path regressions also compile on the pre-family base, which
//! rejects each valid thieve step as an unknown handler.

use api::quest_facts::QuestCatalog;
use api::selected::ClientRevision;
use script::quester::compile::{compile_uncached_for_test, decode_cook, CompileError};
use script::quester::path::PredicateDocument;
use serde_json::{json, Value};

fn compile(args: Value) -> Result<(), CompileError> {
    let _home = script::IsolatedEnv::enter("quester-thieve-family");
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = QuestCatalog::from_identity(selected.quest_identity()).unwrap();
    let mut document = decode_cook().unwrap();
    let step = &mut document.roles[0].sequences[0].steps[0];
    step.kind = "thieve".into();
    step.advances = false.into();
    let mut settle = args["until"].clone();
    if let Some(alias) = settle["obj"]["id"]
        .as_i64()
        .and_then(|id| selected.item_by_id(i32::try_from(id).ok()?))
        .and_then(|item| item.alias.as_ref())
    {
        settle["obj"] = json!(alias);
    }
    step.settle = PredicateDocument::Fact {
        kind: "item_count_at_least".into(),
        version: 1,
        args: settle,
    };
    step.args = args;
    compile_uncached_for_test(&document, &selected, &quests).map(|_| ())
}

#[test]
fn quest_thieve_compiles_selected_man_workmen_and_prison_guards() {
    for npc in [
        "man",
        "digworkman1",
        "digworkman2",
        "troll_prison_guard1",
        "troll_prison_guard2",
    ] {
        compile(json!({
            "target":{"npc":npc},
            "until":{"obj":{"id":995},"qty":1},
            "settle_ms":60000,
        }))
        .unwrap_or_else(|error| panic!("{npc}: {}", error.code));
    }
}

#[test]
fn quest_thieve_rejects_unknown_targets_and_invalid_inventory_goals() {
    for args in [
        json!({"target":{"npc":"unknown quest workman"},"until":{"obj":"coins","qty":1}}),
        json!({"target":{"npc":"man"},"until":{"obj":{"id":-1},"qty":1}}),
        json!({"target":{"npc":"man"},"until":{"obj":"coins","qty":0}}),
        json!({"target":{"npc":"man"},"until":{"obj":"coins","qty":1},"settle_ms":0}),
        json!({"target":{"npc":"man"},"until":{"obj":"coins","qty":1},"bank":true}),
    ] {
        assert!(compile(args).is_err());
    }
}

#[test]
fn quest_thieve_uses_the_shared_provenance_anchor_and_finite_authored_deadline() {
    compile(json!({
        "target":{"npc":"man"},
        "until":{"obj":"coins","qty":1},
        "anchor":{"tile":[3222,3218,0],"source":"unit fixture"},
        "radius":12,
        "settle_ms":600001,
    }))
    .unwrap();
}
