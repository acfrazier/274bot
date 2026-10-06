//! These authored Path regressions also compile on the pre-family base: that
//! base rejects every valid gather step as an unknown family.

use api::game_data::SelectedGameData;
use api::quest_facts::QuestCatalog;
use api::selected::{ClientRevision, FamilyPreparation};
use script::quester::compile::{compile_uncached_for_test, decode_cook, CompileError};
use script::quester::path::PredicateDocument;
use serde_json::{json, Value};
use std::sync::Arc;

fn compile(args: Value) -> Result<(), CompileError> {
    let _home = script::IsolatedEnv::enter("quester-gather-family");
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let catalog = FamilyPreparation::run({
        let selected = Arc::clone(&selected);
        move |worker| api::gather_methods::prepare(&selected, worker)
    })
    .unwrap()
    .join()
    .unwrap()
    .unwrap();
    let mut document = decode_cook().unwrap();
    let step = &mut document.roles[0].sequences[0].steps[0];
    step.kind = "gather".into();
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
    let quests = QuestCatalog::from_identity(selected.quest_identity()).unwrap();
    let result = compile_uncached_for_test(&document, &selected, &quests).map(|_| ());
    drop(catalog);
    result
}

fn id(selected: &SelectedGameData, alias: &str) -> i32 {
    selected.item_by_alias(alias).unwrap().id
}

#[test]
fn quest_gather_compiles_selected_ores_and_karambwanji_with_exact_inventory_goals() {
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    for (skill, resource, alias) in [
        ("mining", "copper", "copper_ore"),
        ("mining", "tin", "tin_ore"),
        ("mining", "iron", "iron_ore"),
        ("fishing", "tbwt_raw_karambwanji", "tbwt_raw_karambwanji"),
    ] {
        compile(json!({"skill":skill,"resource":resource,"until":{"obj":{"id":id(&selected, alias)},"qty":1}}))
            .unwrap_or_else(|error| panic!("{skill}/{resource}: {}", error.code));
    }
}

#[test]
fn quest_gather_compiles_the_content_derived_contest_north_spot_method() {
    compile(json!({
        "skill":"fishing", "method":"fishing.0_41_53_sinisterfishspot.op1",
        "until":{"obj":"raw_giant_carp","qty":2}
    }))
    .unwrap();
}

#[test]
fn quest_gather_rejects_an_unrelated_product_and_unbounded_or_ambiguous_arguments() {
    for args in [
        json!({"skill":"mining","resource":"copper","until":{"obj":"tin_ore","qty":1}}),
        json!({"skill":"mining","resource":"copper","method":"mining.copper","until":{"obj":"copper_ore","qty":1}}),
        json!({"skill":"mining","resource":"copper","until":{"obj":"copper_ore","qty":0}}),
        json!({"skill":"mining","resource":"copper","until":{"obj":"copper_ore","qty":1},"settle_ms":0}),
        json!({"skill":"mining","resource":"copper","until":{"obj":"copper_ore","qty":1},"bank":true}),
    ] {
        assert!(compile(args).is_err());
    }
}
