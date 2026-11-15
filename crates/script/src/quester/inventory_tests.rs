use super::super::{empty_loadout_carry, CompiledProgress};
use super::*;
use crate::quester::loadouts::LoadoutOverlay;
use crate::quester::path::{PathDocument, QuestItemDocument};
use api::quest_facts::QuestCatalog;
use api::selected::{ClientRevision, FactKey};
use serde_json::json;
use std::collections::HashMap;

fn step(kind: &str, args: serde_json::Value) -> StepDocument {
    serde_json::from_value(json!({
        "id": format!("inventory-{kind}"), "kind": kind, "version": 1,
        "args": args, "skip_if": {"All": []}, "settle": {"All": []}
    }))
    .unwrap()
}

fn with_fixture(
    customize: impl FnOnce(&mut PathDocument),
    inspect: impl FnOnce(&QuestHeaderDocument, &PathRoleDocument, &CompileContext<'_>),
) {
    if !api::test_support::game_data_available() {
        return;
    }
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = QuestCatalog::from_identity(selected.quest_identity()).unwrap();
    let mut document = super::super::decode_cook().unwrap();
    let header = document.quest.as_mut().unwrap();
    header.items.clear();
    header.tools.clear();
    header.acquire.clear();
    header.loadouts.clear();
    let role = &mut document.roles[0];
    role.prelude.clear();
    role.progress_reader = None;
    for sequence in &mut role.sequences {
        sequence.steps.clear();
    }
    customize(&mut document);
    let header = document.quest.as_ref().unwrap();
    let role = &document.roles[0];
    let name = |alias: &str| {
        selected
            .item_by_alias(alias)
            .unwrap()
            .name
            .as_deref()
            .unwrap()
    };
    let compiled: Vec<_> = header
        .loadouts
        .iter()
        .map(|(key, row)| {
            let mut loadout = Loadout::new(format!("{}/{key}", document.id.0));
            for (slot, alias) in &row.worn {
                loadout = loadout.with_slot(slot, name(alias));
            }
            for carry in &row.carry {
                loadout = loadout.with_carry(name(&carry.item), carry.qty);
            }
            loadout
        })
        .collect();
    let loadouts = LoadoutOverlay::new(
        Arc::from([Loadout::new("operator/unused").with_slot("hat", name("iron_full_helm"))]),
        compiled.into(),
    );
    let progress = CompiledProgress {
        binding: role.progress_binding.clone(),
        role: None,
        colour_not_started: FactKey::new("not-started"),
        colour_in_progress: FactKey::new("in-progress"),
        colour_complete: FactKey::new("complete"),
        stage_keys: Arc::from([]),
        rules: Arc::from([]),
        flags: Arc::from([]),
        monotonic: true,
    };
    let areas = HashMap::new();
    let recipes = HashMap::new();
    let cx = CompileContext {
        path: &document.id,
        kind: document.kind,
        progress: &progress,
        pair: None,
        selected: &selected,
        quests: &quests,
        gathering: None,
        areas: &areas,
        recipes: &recipes,
        bank: None,
        bank_required: false,
        keep_ids: &[],
        loadouts: &loadouts,
        loadout_carry: empty_loadout_carry(),
    };
    inspect(header, role, &cx);
}

fn id(cx: &CompileContext<'_>, alias: &str) -> i32 {
    cx.selected.item_by_alias(alias).unwrap().id
}

fn quantity(inventory: &StageInventory, cx: &CompileContext<'_>, alias: &str) -> Option<i32> {
    let id = id(cx, alias);
    inventory
        .items
        .iter()
        .find(|row| row.item.id == id)
        .map(|row| row.qty)
}

#[test]
fn managed_items_are_narrow_sorted_and_include_unused_recipe_inputs_and_outputs() {
    with_fixture(
        |document| {
            let header = document.quest.as_mut().unwrap();
            header.items = vec![
                serde_json::from_value::<QuestItemDocument>(
                    json!({"obj": "bucket_empty", "qty": 1, "kind": "must_have"}),
                )
                .unwrap(),
                serde_json::from_value::<QuestItemDocument>(
                    json!({"obj": "bread", "qty": 2, "kind": "acquirable"}),
                )
                .unwrap(),
            ];
            header.tools.push("obj:tinderbox".into());
            header.loadouts.insert(
                "guide".into(),
                serde_json::from_value(json!({
                    "worn": {"hat": "bronze_full_helm"}, "carry": [{"item": "lobster", "qty": 3}]
                }))
                .unwrap(),
            );
            header.acquire.insert("unused".into(), vec![
            step("use_on", json!({"item": "bronze_sword", "target": {"item": "bat_bones"}, "product": "bronze_dart_tip"})),
        ]);
            document.roles[0].sequences[0].steps.push(step(
                "bank",
                json!({
                    "op": "withdraw", "items": [{"obj": "coins", "qty": 50}]
                }),
            ));
        },
        |header, role, cx| {
            let (managed, carried) = compile_managed_items(header, role, cx).unwrap();
            let mut expected: Vec<_> = [
                "bucket_empty",
                "bread",
                "tinderbox",
                "bronze_full_helm",
                "lobster",
                "bronze_sword",
                "bat_bones",
                "bronze_dart_tip",
                "coins",
            ]
            .into_iter()
            .map(|alias| id(cx, alias))
            .collect();
            expected.sort_unstable();
            assert_eq!(managed.as_ref(), expected);
            assert_eq!(carried.len(), 2);
            assert!(carried.contains(&id(cx, "bronze_sword")));
            assert!(carried.contains(&id(cx, "bat_bones")));
            assert!(
                !managed.contains(&id(cx, "iron_full_helm")),
                "unrelated operator row"
            );
            assert!(
                !managed.contains(&id(cx, "bronze_axe")),
                "unrelated global tool family"
            );
            let stage = stage_inventory(role.sequences[0].steps.iter(), header, cx).unwrap();
            assert_eq!(
                stage.equipment_slots, 0,
                "unused guide gear does not claim a slot"
            );
        },
    );
}

#[test]
fn carried_use_and_pack_predicates_are_distinct_from_banked_guide_gear() {
    with_fixture(
        |document| {
            document.roles[0].sequences[0].steps = vec![
                step(
                    "use_on",
                    json!({"item": "bronze_sword", "target": {"item": "iron_full_helm"}}),
                ),
                step(
                    "interact",
                    json!({"target": {"held": "tinderbox"}, "op": "Use"}),
                ),
                step(
                    "partner",
                    json!({"give": [{"obj": "bread", "qty": 1}], "take": []}),
                ),
                step(
                    "wait",
                    json!({"until": {"Fact": {"kind": "has_item", "version": 1, "args": {"obj": "bat_bones"}}}}),
                ),
                step(
                    "bank",
                    json!({"op": "withdraw", "items": [{"obj": "bronze_full_helm", "qty": 1}]}),
                ),
            ];
        },
        |header, role, cx| {
            let (managed, carried) = compile_managed_items(header, role, cx).unwrap();
            for alias in [
                "bronze_sword",
                "iron_full_helm",
                "tinderbox",
                "bread",
                "bat_bones",
            ] {
                assert!(carried.contains(&id(cx, alias)), "{alias}");
            }
            assert!(managed.contains(&id(cx, "bronze_full_helm")));
            assert!(!carried.contains(&id(cx, "bronze_full_helm")));
        },
    );
}

#[test]
fn all_declared_output_families_reserve_selected_stackability_and_maximum_targets() {
    with_fixture(
        |document| {
            document.roles[0].sequences[0].steps = vec![
                step(
                    "bank",
                    json!({"op": "withdraw", "items": [{"obj": "bucket_empty", "qty": 27}]}),
                ),
                step(
                    "bank",
                    json!({"op": "withdraw", "items": [{"obj": "bucket_empty", "qty": 3}]}),
                ),
                step(
                    "bank",
                    json!({"op": "deposit", "items": [{"obj": "iron_full_helm", "qty": 1}]}),
                ),
                step("buy", json!({"obj": "bread", "qty": 2})),
                step("make", json!({"product": "bronze_dart_tip", "qty": 10})),
                step(
                    "use_on",
                    json!({"item": "tinderbox", "target": {"item": "bread"}, "product": "lobster"}),
                ),
                step(
                    "interact",
                    json!({"target": {"ground": "bat_bones"}, "op": "Take"}),
                ),
                step("gather", json!({"until": {"obj": "coins", "qty": 1000}})),
                step("thieve", json!({"until": {"obj": {"id": 995}, "qty": 50}})),
                step(
                    "combat",
                    json!({"loot": [{"obj": "bronze_sword", "qty": 2}], "until_drop": {"obj": "bronze_full_helm"}}),
                ),
            ];
        },
        |header, role, cx| {
            // Resolve the exact-ID selector from selected content, not an assumed revision ID.
            let mut steps = role.sequences[0].steps.clone();
            steps[8].args["until"]["obj"]["id"] = json!(id(cx, "coins"));
            let stage = stage_inventory(steps.iter(), header, cx).unwrap();
            for (alias, qty) in [
                ("bucket_empty", 27),
                ("bread", 2),
                ("bronze_dart_tip", 10),
                ("lobster", 1),
                ("bat_bones", 1),
                ("coins", 1000),
                ("bronze_sword", 2),
                ("bronze_full_helm", 1),
            ] {
                assert_eq!(quantity(&stage, cx, alias), Some(qty), "{alias}");
            }
            assert_eq!(
                quantity(&stage, cx, "iron_full_helm"),
                None,
                "deposits do not need future room"
            );
            for alias in ["coins", "bronze_dart_tip"] {
                assert!(
                    stage
                        .items
                        .iter()
                        .find(|row| row.item.id == id(cx, alias))
                        .unwrap()
                        .stackable
                );
            }
            assert!(
                !stage
                    .items
                    .iter()
                    .find(|row| row.item.id == id(cx, "bucket_empty"))
                    .unwrap()
                    .stackable
            );
        },
    );
}

#[test]
fn nested_recipes_reserve_only_reachable_outputs_once_per_held_target() {
    with_fixture(
        |document| {
            let header = document.quest.as_mut().unwrap();
            header.items.push(
                serde_json::from_value(
                    json!({"obj": "bread", "qty": 4, "kind": "acquirable", "acquire": "inner"}),
                )
                .unwrap(),
            );
            header.acquire.insert(
                "inner".into(),
                vec![step("make", json!({"product": "bread", "qty": 4}))],
            );
            header.acquire.insert(
                "outer".into(),
                vec![
                    step("acquire", json!({"recipe": "inner"})),
                    step(
                        "bank",
                        json!({"op": "withdraw", "items": [{"obj": "bucket_empty", "qty": 3}]}),
                    ),
                ],
            );
            header.acquire.insert(
                "unreached".into(),
                vec![step("buy", json!({"obj": "lobster", "qty": 2}))],
            );
            document.roles[0].sequences[0].steps = vec![
                step("acquire", json!({"recipe": "outer"})),
                step("acquire", json!({"recipe": "outer"})),
            ];
        },
        |header, role, cx| {
            let stage = stage_inventory(role.sequences[0].steps.iter(), header, cx).unwrap();
            assert_eq!(quantity(&stage, cx, "bread"), Some(4));
            assert_eq!(quantity(&stage, cx, "bucket_empty"), Some(3));
            assert_eq!(quantity(&stage, cx, "lobster"), None);
            let (managed, _) = compile_managed_items(header, role, cx).unwrap();
            assert!(
                managed.contains(&id(cx, "lobster")),
                "all declared recipes remain protected"
            );
        },
    );
}

#[test]
fn dynamic_quantities_reserve_pack_limit_and_keep_the_subtracted_item_carried() {
    with_fixture(
        |document| {
            document.roles[0].sequences[0].steps.push(step("make", json!({
            "product": "bronze_dart_tip", "qty": {"progress": {"quest": "cook", "flag": "count"}, "minus_item": "bread"}
        })));
        },
        |header, role, cx| {
            let stage = stage_inventory(role.sequences[0].steps.iter(), header, cx).unwrap();
            assert_eq!(quantity(&stage, cx, "bronze_dart_tip"), Some(28));
            let (_, carried) = compile_managed_items(header, role, cx).unwrap();
            assert!(carried.contains(&id(cx, "bread")));
        },
    );
}

#[test]
fn authored_equipment_claims_include_secondary_slots_and_strict_empty_equipment() {
    with_fixture(
        |document| {
            document.roles[0].sequences[0]
                .steps
                .push(step("equip", json!({"obj": "bronze_2h_sword"})));
        },
        |header, role, cx| {
            let stage = stage_inventory(role.sequences[0].steps.iter(), header, cx).unwrap();
            for slot in ["righthand", "lefthand"] {
                let position = game_data::wearpos_for_loadout_slot(slot).unwrap();
                assert_ne!(stage.equipment_slots & (1 << position), 0, "{slot}");
            }
            let mut strict = step("wait", json!({}));
            strict.settle = serde_json::from_value(
                json!({"Fact": {"kind": "equipment_only", "version": 1, "args": {"objs": []}}}),
            )
            .unwrap();
            let stage = stage_inventory(std::iter::once(&strict), header, cx).unwrap();
            for slot in api::loadout::WORN_SLOTS {
                let position = game_data::wearpos_for_loadout_slot(slot).unwrap();
                assert_ne!(stage.equipment_slots & (1 << position), 0, "{slot}");
            }
        },
    );
}
