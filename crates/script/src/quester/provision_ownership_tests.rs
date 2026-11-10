use super::tests::{
    bank_row, complete_bank_pick, complete_open_stand, item_view, openable_bank_fixture,
    poll_once_at, snapshot_view, survival_fixture_document, survival_input,
};
use super::*;
use crate::interact::InteractReq;
use crate::native::{ledger, HostEffect};
use crate::quester::compile::CompiledPath;
use crate::quester::families::tests::with_tick_bank;
use crate::quester::path::{PathDocument, PredicateDocument, StepDocument};
use api::bank_memory::BankMemory;
use api::quest_facts::QuestCatalog;
use api::selected::ClientRevision;
use api::snapshot::{GameSnapshot, ItemContainer};

struct Prepared {
    provisioner: Provisioner,
    withdrawn: Vec<(i32, i32)>,
    deposited: Vec<i32>,
    equipped: Vec<i32>,
}

fn item(alias: &str, slot: i32, container: ItemContainer) -> ItemView {
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let fact = selected.item_by_alias(alias).unwrap();
    let mut row = item_view(fact.id, fact.name.as_deref().unwrap(), 1, container);
    row.slot = slot;
    row.def.stackable = fact.stackable;
    row
}

fn bank_items(gear: bool) -> Vec<ItemView> {
    let mut rows = vec![item("lobster", 0, ItemContainer::Bank)];
    rows[0].count = 28;
    if gear {
        for alias in [
            "bronze_full_helm",
            "bronze_platebody",
            "bronze_platelegs",
            "bronze_kiteshield",
            "bronze_sword",
        ] {
            rows.push(item(alias, rows.len() as i32, ItemContainer::Bank));
        }
    }
    for row in &mut rows {
        row.action_family = api::snapshot::ItemActionFamily::Component;
        row.component_id = 7;
        row.actions = vec![Some("Withdraw-1".into())];
    }
    rows
}

fn side_rows(pack: &[ItemView]) -> Vec<ItemView> {
    pack.iter()
        .map(|row| ItemView {
            container: ItemContainer::BankSide,
            action_family: api::snapshot::ItemActionFamily::Component,
            component_id: 8,
            actions: vec![Some("Deposit-1".into())],
            ..row.clone()
        })
        .collect()
}

fn prepare(
    path: &CompiledPath,
    quests: &QuestCatalog,
    snapshot: &mut GameSnapshot,
    mut rows: Vec<ItemView>,
) -> Prepared {
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let (_, bank, banks) = openable_bank_fixture();
    let (mut pack, mut worn) = {
        let view = snapshot_view(snapshot, 0);
        (
            view.inventory().unwrap().value.to_vec(),
            view.equipment().unwrap().value.to_vec(),
        )
    };
    snapshot.seed_bank_observation(1, 0, Some(rows.clone()), side_rows(&pack));
    snapshot.seed_inventory_packet_generations(Some(0), Some(0));
    let mut memory = BankMemory::default();
    let mut ledger: Option<Box<ledger::Ledger>> = None;
    let mut observed = Prepared {
        provisioner: Provisioner::new(),
        withdrawn: Vec::new(),
        deposited: Vec::new(),
        equipped: Vec::new(),
    };
    for tick in 1..=240 {
        memory.track(snapshot, tick);
        // Match survival_input's known-clear fixture. The default native test
        // context has no poison fact and may legitimately keep an optional kit.
        let condition = path.provisioning.survival.as_ref().map(|survival| {
            let mut condition =
                crate::survival::condition::derive(snapshot, survival.tables()).unwrap();
            condition.poison = api::condition::PoisonState::Clear;
            condition
        });
        let result = with_tick_bank(snapshot, Some(&memory), &mut ledger, tick, |native| {
            native.cx.condition = condition;
            let required_after = native.cx.evidence();
            let mut cx = StepContext {
                tick: native,
                quests,
                progress: &[],
                required_after,
                banks: &banks,
                choices: &crate::quester::choices::QuestChoices::default(),
            };
            observed.provisioner.poll(
                &mut cx,
                &path.provisioning,
                path.provisioning
                    .loadout_carry
                    .keys()
                    .next()
                    .map(AsRef::as_ref),
                Some(&path.provisioning.stages[0]),
            )
        });
        let effect = ledger
            .as_ref()
            .and_then(|ledger| ledger.outbox.first())
            .map(|action| &action.effect);
        if let Some(effect) = effect {
            match effect {
                HostEffect::BankPick(_) => {
                    complete_bank_pick(&mut ledger, bank, bank.tile, tick);
                }
                HostEffect::Interaction(InteractReq::OpenStand { .. }) => {
                    complete_open_stand(&mut ledger, tick);
                    snapshot.seed_bank_observation(1, tick, Some(rows.clone()), side_rows(&pack));
                }
                HostEffect::Interaction(InteractReq::Withdraw { name, action }) => {
                    let count = action.strip_prefix("Withdraw-").unwrap().parse().unwrap();
                    withdraw(name, count, &mut rows, &mut pack, &mut observed.withdrawn);
                    complete_open_stand(&mut ledger, tick);
                }
                HostEffect::Interaction(InteractReq::WithdrawX { name, count, .. }) => {
                    withdraw(name, *count, &mut rows, &mut pack, &mut observed.withdrawn);
                    complete_open_stand(&mut ledger, tick);
                }
                HostEffect::Interaction(InteractReq::InvButton {
                    id,
                    slot,
                    operation,
                    ..
                }) => {
                    assert!(!path.managed_item_ids().contains(id), "managed deposit");
                    assert_eq!(*operation, 1, "the fixture posts only Deposit-1");
                    let index = pack
                        .iter()
                        .position(|row| row.def.id == *id && row.slot == *slot)
                        .expect("deposit names a held row");
                    let row = pack.remove(index);
                    assert_eq!(row.count, 1);
                    observed.deposited.push(*id);
                    rows.push(bank_row(*id, row.def.name.as_deref().unwrap(), 1));
                    complete_open_stand(&mut ledger, tick);
                }
                HostEffect::Interaction(InteractReq::Wear { name }) => {
                    let index = pack
                        .iter()
                        .position(|row| row.def.name.as_deref() == Some(name.as_str()))
                        .expect("survival gear is held before wear");
                    let mut row = pack.remove(index);
                    assert!(!path.provisioning.carried_items.contains(&row.def.id));
                    let fact = selected.item_by_id(row.def.id).unwrap();
                    let positions = [
                        fact.wear_position,
                        fact.wear_position_2,
                        fact.wear_position_3,
                    ];
                    let mut index = 0;
                    while index < worn.len() {
                        if positions.contains(&worn[index].slot) {
                            assert!(
                                !path.managed_item_ids().contains(&worn[index].def.id),
                                "survival wear must not unequip managed gear"
                            );
                            let mut displaced = worn.remove(index);
                            displaced.container = ItemContainer::Inventory;
                            displaced.slot = free_slot(&pack);
                            pack.push(displaced);
                        } else {
                            index += 1;
                        }
                    }
                    observed.equipped.push(row.def.id);
                    row.container = ItemContainer::Equipment;
                    row.slot = fact.wear_position;
                    worn.push(row);
                    complete_open_stand(&mut ledger, tick);
                    snapshot.seed_bank_observation(-1, tick, None, Vec::new());
                }
                HostEffect::Interaction(InteractReq::SetNoteMode { .. }) => {
                    complete_open_stand(&mut ledger, tick);
                }
                HostEffect::Interaction(InteractReq::Close) => {
                    complete_open_stand(&mut ledger, tick);
                    snapshot.seed_bank_observation(-1, tick, None, Vec::new());
                }
                _ => panic!("owned survival emitted a quest or unrelated effect"),
            }
            snapshot.seed_inventory(pack.clone(), 28);
            snapshot.seed_inventory_packet_generations(Some(tick), Some(tick));
            snapshot.seed_equipment(worn.clone());
            if snapshot_view(snapshot, tick)
                .bank_session()
                .is_some_and(|session| session.value.open)
            {
                snapshot.seed_bank_observation(1, tick, Some(rows.clone()), side_rows(&pack));
            }
        }
        match result {
            Poll::Ready(Ok(ProvisionEvent::Ready)) => return observed,
            Poll::Pending | Poll::Ready(Ok(ProvisionEvent::BankReceipt(_))) => {}
            other => panic!("owned survival must remain informational: {other:?}"),
        }
    }
    panic!(
        "owned survival did not finish: {:?}",
        observed.provisioner.status.phase
    );
}

fn free_slot(pack: &[ItemView]) -> i32 {
    (0..28)
        .find(|slot| pack.iter().all(|row| row.slot != *slot))
        .expect("the real pack must have room before a withdrawal")
}

fn withdraw(
    name: &str,
    count: i32,
    rows: &mut [ItemView],
    pack: &mut Vec<ItemView>,
    withdrawn: &mut Vec<(i32, i32)>,
) {
    let row = rows
        .iter_mut()
        .find(|row| row.def.name.as_deref() == Some(name))
        .expect("withdrawal names bank stock");
    assert!(count > 0 && count <= row.count);
    withdrawn.push((row.def.id, count));
    for _ in 0..count {
        let mut incoming = item_view(row.def.id, name, 1, ItemContainer::Inventory);
        incoming.slot = free_slot(pack);
        pack.push(incoming);
    }
    row.count -= count;
}

fn extra_step(document: &mut PathDocument, kind: &str, args: serde_json::Value) {
    let mut step: StepDocument = document.roles[0].sequences[0].steps[0].clone();
    step.id = FactKey::new(&format!("owned-{kind}"));
    step.kind = kind.into();
    step.args = args;
    step.advances = Some(false);
    step.skip_if = PredicateDocument::Any(Vec::new());
    step.settle = PredicateDocument::All(Vec::new());
    document.roles[0].sequences[0].steps.insert(0, step);
}

fn owned(document: &mut PathDocument) {
    document.quest.as_mut().unwrap().owns_inventory = true;
}

fn food_count(observed: &Prepared) -> i32 {
    let id = item("lobster", 0, ItemContainer::Inventory).def.id;
    observed
        .withdrawn
        .iter()
        .filter(|&&(item, _)| item == id)
        .map(|&(_, count)| count)
        .sum()
}

#[test]
fn owned_inventory_still_withdraws_food_and_passes_the_bound() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (path, quests, mut snapshot) = survival_fixture_document(true, false, owned);
    let observed = prepare(&path, &quests, &mut snapshot, bank_items(false));
    assert!(food_count(&observed) > 0);
    assert!(observed.deposited.is_empty());
    assert!(observed.equipped.is_empty());
    let survival = path.provisioning.survival.as_ref().unwrap();
    let mut memory = BankMemory::default();
    memory.track(&snapshot, 300);
    let need = survival
        .plan(
            0,
            Stock {
                bank: Some(&memory),
                ..snapshot_view(&snapshot, 300).stock()
            },
            survival_input(&snapshot, survival),
            Some(&path.provisioning),
        )
        .unwrap();
    assert!(need.bound <= crate::combat::risk::consts::DEATH_BOUND);
}

#[test]
fn owned_inventory_protects_managed_pack_items_but_can_deposit_unrelated_cargo() {
    if !api::test_support::game_data_available() {
        return;
    }
    for unrelated in [false, true] {
        let (path, quests, mut snapshot) = survival_fixture_document(true, false, |document| {
            owned(document);
            document
                .quest
                .as_mut()
                .unwrap()
                .tools
                .push("obj:bucket_empty".into());
        });
        let mut pack: Vec<_> = (0..28)
            .map(|slot| item("bucket_empty", slot, ItemContainer::Inventory))
            .collect();
        let junk_id = 900_000;
        if unrelated {
            pack[27] = item_view(junk_id, "Unrelated cargo", 1, ItemContainer::Inventory);
            pack[27].slot = 27;
        }
        snapshot.seed_inventory(pack, 28);
        let observed = prepare(&path, &quests, &mut snapshot, bank_items(false));
        if unrelated {
            assert_eq!(observed.deposited, [junk_id]);
            assert_eq!(food_count(&observed), 1);
        } else {
            assert!(observed.deposited.is_empty());
            assert!(observed.withdrawn.is_empty());
        }
        let bucket = item("bucket_empty", 0, ItemContainer::Inventory).def.id;
        assert_eq!(
            ops::count_id(
                snapshot_view(&snapshot, 300).inventory().unwrap().value,
                bucket
            ),
            if unrelated { 27 } else { 28 }
        );
        assert!(observed.provisioner.unmet_logged.iter().any(|need| {
            matches!(need.cause, UnmetCause::NoRoom)
                && need.need > need.have
                && need.have == i32::from(unrelated)
        }));
        assert_eq!(observed.provisioner.status().phase, ProvisionPhase::Ready);
    }
}

#[test]
fn owned_food_reserves_future_withdrawals_without_executing_them() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (baseline_path, quests, mut baseline_snapshot) =
        survival_fixture_document(true, false, owned);
    let baseline = prepare(
        &baseline_path,
        &quests,
        &mut baseline_snapshot,
        bank_items(false),
    );
    assert!(food_count(&baseline) > 1);
    for (alias, quantity, reserved_slots) in [("bucket_empty", 27, 27), ("coins", 1000, 1)] {
        let (path, quests, mut snapshot) = survival_fixture_document(true, false, |document| {
            owned(document);
            extra_step(
                document,
                "bank",
                serde_json::json!({
                    "op": "withdraw", "items": [{"obj": alias, "qty": quantity}]
                }),
            );
        });
        let observed = prepare(&path, &quests, &mut snapshot, bank_items(false));
        assert_eq!(
            food_count(&observed),
            food_count(&baseline).min(28 - reserved_slots)
        );
        let lobster = item("lobster", 0, ItemContainer::Inventory).def.id;
        assert!(observed.withdrawn.iter().all(|&(id, _)| id == lobster));
        assert!(observed.deposited.is_empty());
    }
}

#[test]
fn owned_gear_respects_authored_slots_and_carried_item_use() {
    if !api::test_support::game_data_available() {
        return;
    }
    for restriction in ["none", "slot", "future-slot", "carried", "worn"] {
        let (path, quests, mut snapshot) = survival_fixture_document(true, true, |document| {
            owned(document);
            document.roles[0].sequences[0].steps[0]
                .args
                .as_object_mut()
                .unwrap()
                .remove("loadout");
            match restriction {
                "slot" => extra_step(
                    document,
                    "equip",
                    serde_json::json!({"obj": "iron_full_helm"}),
                ),
                "future-slot" => {
                    extra_step(
                        document,
                        "equip",
                        serde_json::json!({"obj": "iron_full_helm"}),
                    );
                    let step = document.roles[0].sequences[0].steps.remove(0);
                    document.roles[0].sequences[1].steps.insert(0, step);
                }
                "carried" => extra_step(
                    document,
                    "wait",
                    serde_json::json!({
                        "until": {"Fact": {"kind": "has_item", "version": 1, "args": {"obj": "bronze_sword"}}},
                        "max_ticks": 1
                    }),
                ),
                "worn" => document
                    .quest
                    .as_mut()
                    .unwrap()
                    .tools
                    .push("obj:bronze_dagger".into()),
                _ => {}
            }
        });
        if restriction == "carried" {
            snapshot.seed_inventory(vec![item("bronze_sword", 0, ItemContainer::Inventory)], 28);
        }
        if restriction == "worn" {
            // Zero melee defence keeps the free armour slots useful, rather
            // than testing a legitimate tie with an already adequate helmet.
            snapshot.seed_equipment(vec![item("bronze_dagger", 3, ItemContainer::Equipment)]);
        }
        let observed = prepare(&path, &quests, &mut snapshot, bank_items(true));
        let helm = item("bronze_full_helm", 0, ItemContainer::Inventory).def.id;
        let sword = item("bronze_sword", 0, ItemContainer::Inventory).def.id;
        match restriction {
            "none" => {
                assert!(observed.equipped.contains(&helm));
                assert!(observed.equipped.contains(&sword));
            }
            "slot" | "future-slot" => assert!(!observed.equipped.contains(&helm)),
            "carried" => {
                assert!(!observed.equipped.contains(&sword));
                assert_eq!(
                    ops::count_id(
                        snapshot_view(&snapshot, 300).inventory().unwrap().value,
                        sword
                    ),
                    1
                );
            }
            "worn" => {
                assert!(!observed.equipped.contains(&sword));
                assert_eq!(
                    ops::count_id(
                        snapshot_view(&snapshot, 300).equipment().unwrap().value,
                        item("bronze_dagger", 3, ItemContainer::Equipment).def.id
                    ),
                    1
                );
            }
            _ => unreachable!(),
        }
        assert!(
            !observed.equipped.is_empty(),
            "{restriction}: unclaimed slots remain available"
        );
    }
}

#[test]
fn owned_inventory_leaves_missing_quest_items_and_authored_carry_to_the_path() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (path, quests, mut snapshot) = survival_fixture_document(true, false, |document| {
        owned(document);
        let header = document.quest.as_mut().unwrap();
        header.coin_float = 12;
        header.loadouts.insert(
            "path-carry".into(),
            crate::quester::path::QuestLoadoutDocument {
                worn: Default::default(),
                carry: vec![crate::quester::path::LoadoutCarryDocument {
                    item: "bucket_empty".into(),
                    qty: 1,
                }],
            },
        );
        header.items.push(crate::quester::path::QuestItemDocument {
            obj: "bucket_empty".into(),
            qty: 1,
            kind: crate::quester::path::QuestItemKindDocument::MustHave,
            acquire: None,
            from_stage: None,
        });
    });
    let observed = prepare(&path, &quests, &mut snapshot, bank_items(false));
    assert!(food_count(&observed) > 0);
    assert!(observed.provisioner.unmet_logged.is_empty());
    let lobster = item("lobster", 0, ItemContainer::Inventory).def.id;
    assert!(observed.withdrawn.iter().all(|&(id, _)| id == lobster));
}

#[test]
fn arthur_reports_the_compiled_quest_managed_item_set() {
    if !api::test_support::game_data_available() {
        return;
    }
    let Some(nav) = crate::quester::compile::real_nav_lease(ClientRevision::R289) else {
        return;
    };
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = QuestCatalog::from_identity(selected.quest_identity()).unwrap();
    let document = serde_json::from_str(include_str!("../../paths/289/arthur.json")).unwrap();
    let path = crate::quester::compile::compile_uncached_for_test_with_zones(
        &document,
        &selected,
        &quests,
        nav.zones().unwrap(),
    )
    .unwrap();
    for alias in [
        "bread",
        "bucket_empty",
        "insect_repellent",
        "bucket_wax",
        "bat_bones",
        "tinderbox",
        "unlit_black_candle",
        "lit_black_candle",
        "excalibur",
    ] {
        assert!(path
            .managed_item_ids()
            .contains(&selected.item_by_alias(alias).unwrap().id));
    }
    assert!(!path
        .managed_item_ids()
        .contains(&selected.item_by_alias("bronze_pickaxe").unwrap().id));
    let named: Vec<_> = path
        .managed_item_ids()
        .iter()
        .map(|&id| {
            let item = selected.item_by_id(id).unwrap();
            (id, item.name.as_deref().unwrap())
        })
        .collect();
    eprintln!("ARTHUR_MANAGED_ITEMS={named:?}");
}

#[test]
fn owned_quest_food_stays_carried_while_bank_survival_food_is_withdrawn() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (path, quests, mut snapshot) = survival_fixture_document(true, false, |document| {
        owned(document);
        document
            .quest
            .as_mut()
            .unwrap()
            .items
            .push(crate::quester::path::QuestItemDocument {
                obj: "bread".into(),
                qty: 1,
                kind: crate::quester::path::QuestItemKindDocument::MustHave,
                acquire: None,
                from_stage: None,
            });
        extra_step(
            document,
            "wait",
            serde_json::json!({
                "until": {"Fact": {"kind": "has_item", "version": 1, "args": {"obj": "bread"}}},
                "max_ticks": 1
            }),
        );
    });
    let bread = item("bread", 0, ItemContainer::Inventory);
    let bread_id = bread.def.id;
    snapshot.seed_inventory(vec![bread], 28);
    let mut bank = bank_items(false);
    bank[0].count = 8;
    snapshot.seed_bank_observation(1, 0, Some(bank.clone()), Vec::new());
    let mut memory = BankMemory::default();
    memory.track(&snapshot, 0);
    let model = path.provisioning.survival.as_ref().unwrap();
    let expected = model
        .plan(
            0,
            api::stock::Stock {
                bank: Some(&memory),
                ..snapshot_view(&snapshot, 0).stock()
            },
            survival_input(&snapshot, model),
            Some(&path.provisioning),
        )
        .unwrap();
    assert!(
        expected.unknown.is_none(),
        "quest bread is not a model unknown"
    );
    let target = expected.food.as_ref().unwrap().1;
    let observed = prepare(&path, &quests, &mut snapshot, bank);
    assert_eq!(food_count(&observed), target.min(8));
    assert!(observed.deposited.is_empty());
    assert_eq!(
        ops::count_id(
            snapshot_view(&snapshot, 300).inventory().unwrap().value,
            bread_id
        ),
        1
    );
    assert_eq!(
        observed.provisioner.unmet_logged.len(),
        usize::from(target > 8)
    );
    if target > 8 {
        let need = &observed.provisioner.unmet_logged[0];
        assert_eq!(need.cause, UnmetCause::NoSupply);
        assert_eq!((need.need, need.have), (target, 8));
    }
}

#[test]
fn must_have_food_reserves_only_the_declared_quantity_in_survival_targets() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut baseline = None;
    for reserved in [0, 4] {
        let (path, quests, mut snapshot) = survival_fixture_document(true, false, |document| {
            owned(document);
            if reserved > 0 {
                document.quest.as_mut().unwrap().items.push(
                    crate::quester::path::QuestItemDocument {
                        obj: "lobster".into(),
                        qty: reserved as u32,
                        kind: crate::quester::path::QuestItemKindDocument::MustHave,
                        acquire: None,
                        from_stage: None,
                    },
                );
            }
        });
        let held = reserved + 4;
        snapshot.seed_inventory(
            (0..held)
                .map(|slot| item("lobster", slot, ItemContainer::Inventory))
                .collect(),
            28,
        );
        let model = path.provisioning.survival.as_ref().unwrap();
        let need = model
            .plan(
                0,
                snapshot_view(&snapshot, 0).stock(),
                survival_input(&snapshot, model),
                Some(&path.provisioning),
            )
            .unwrap();
        assert!(need.unknown.is_none());
        let target = need.food.as_ref().unwrap().1;
        if let Some((ordinary_target, bound)) = baseline {
            assert_eq!(target, reserved + ordinary_target.min(28 - reserved));
            assert_eq!(
                need.bound, bound,
                "only four surplus lobsters earn food credit"
            );
            let observed = prepare(&path, &quests, &mut snapshot, bank_items(false));
            assert_eq!(food_count(&observed), (target - held).max(0));
        } else {
            baseline = Some((target, need.bound));
        }
    }
}

#[test]
fn two_held_ordinary_food_kinds_report_the_existing_model_limit() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (path, _quests, mut snapshot) = survival_fixture_document(true, false, owned);
    snapshot.seed_inventory(
        vec![
            item("lobster", 0, ItemContainer::Inventory),
            item("trout", 1, ItemContainer::Inventory),
        ],
        28,
    );
    let bank = bank_items(false);
    snapshot.seed_bank_observation(1, 0, Some(bank), Vec::new());
    let mut memory = BankMemory::default();
    memory.track(&snapshot, 0);
    let model = path.provisioning.survival.as_ref().unwrap();
    let target = model
        .plan(
            0,
            api::stock::Stock {
                bank: Some(&memory),
                ..snapshot_view(&snapshot, 0).stock()
            },
            survival_input(&snapshot, model),
            Some(&path.provisioning),
        )
        .unwrap();
    assert_eq!(
        target.unknown,
        Some(crate::combat::risk::UnknownWhy::MixedFood)
    );
    assert!(target.food.is_some());
}

#[test]
fn held_message_delay_food_reports_the_existing_timing_limit() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (path, _quests, mut snapshot) = survival_fixture_document(true, false, owned);
    snapshot.seed_inventory(
        vec![item("tbwt_cooked_karambwan", 0, ItemContainer::Inventory)],
        28,
    );
    let bank = bank_items(false);
    snapshot.seed_bank_observation(1, 0, Some(bank), Vec::new());
    let mut memory = BankMemory::default();
    memory.track(&snapshot, 0);
    let model = path.provisioning.survival.as_ref().unwrap();
    let target = model
        .plan(
            0,
            api::stock::Stock {
                bank: Some(&memory),
                ..snapshot_view(&snapshot, 0).stock()
            },
            survival_input(&snapshot, model),
            Some(&path.provisioning),
        )
        .unwrap();
    assert_eq!(
        target.unknown,
        Some(crate::combat::risk::UnknownWhy::UnsupportedFood)
    );
    assert!(target.food.is_some());
}

#[test]
fn protected_food_is_excluded_before_input_validation_and_the_food_row_cap() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (path, _quests, mut snapshot) = survival_fixture_document(true, false, owned);
    let model = path.provisioning.survival.as_ref().unwrap();
    let mut foods: Vec<_> = model
        .tables()
        .selected()
        .items()
        .iter()
        .filter_map(|item| {
            model
                .tables()
                .food(item.id)
                .filter(|food| food.heal >= 0)
                .map(|food| (item, food.heal))
        })
        .collect();
    foods.sort_by_key(|(item, heal)| (std::cmp::Reverse(*heal), item.id));
    assert!(foods.len() > 6);
    let protected: Vec<_> = foods[..6]
        .iter()
        .map(|(item, _)| crate::combat::FoodReserve {
            id: item.id,
            qty: 1,
        })
        .collect();
    let (ordinary, _) = foods.last().unwrap();
    let mut pack: Vec<_> = foods[..6]
        .iter()
        .map(|(item, _)| {
            item_view(
                item.id,
                item.name.as_deref().unwrap_or("Food"),
                1,
                ItemContainer::Inventory,
            )
        })
        .collect();
    pack.push(item_view(
        ordinary.id,
        ordinary.name.as_deref().unwrap_or("Food"),
        1,
        ItemContainer::Inventory,
    ));
    for (slot, row) in pack.iter_mut().enumerate() {
        row.slot = slot as i32;
    }
    snapshot.seed_inventory(pack.clone(), 28);
    let capture = |snapshot: &GameSnapshot, excluded: &[crate::combat::FoodReserve]| {
        let frame = crate::combat::frame::Frame::borrow(snapshot_view(snapshot, 1)).unwrap();
        crate::combat::risk::RiskInput::capture_excluding(
            &frame,
            model.tables(),
            &crate::combat::ThreatSet::default(),
            api::condition::PoisonState::Clear,
            false,
            excluded,
        )
    };
    let all = capture(&snapshot, &[]);
    assert_eq!(all.food_len, 6);
    assert!(all.more_food());
    let filtered = capture(&snapshot, &protected);
    assert_eq!(filtered.food_len, 1);
    assert_eq!(filtered.food_ids[0], ordinary.id);
    assert!(!filtered.more_food());
    assert_eq!(
        filtered.free_slots, all.free_slots,
        "protected food still occupies slots"
    );

    let mut unknown = item_view(i32::MAX, "Unpriced quest food", 1, ItemContainer::Inventory);
    unknown.actions = vec![Some("Eat".to_owned())];
    let mut ordinary = pack.pop().unwrap();
    ordinary.slot = 1;
    snapshot.seed_inventory(vec![unknown, ordinary], 28);
    assert!(capture(&snapshot, &[]).missing_facts());
    assert!(!capture(
        &snapshot,
        &[crate::combat::FoodReserve {
            id: i32::MAX,
            qty: 1,
        }]
    )
    .missing_facts());
}

#[test]
fn owned_safe_path_does_not_require_inventory_evidence() {
    if !api::test_support::game_data_available() {
        return;
    }
    for owns_inventory in [false, true] {
        let (path, quests, _) = survival_fixture_document(false, false, |document| {
            document.quest.as_mut().unwrap().owns_inventory = owns_inventory;
        });
        assert!(path.provisioning.survival.is_none());
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        let (_, _, banks) = openable_bank_fixture();
        let mut provisioner = Provisioner::new();
        let result = poll_once_at(
            &mut provisioner,
            &snapshot,
            &mut None,
            1,
            &path.provisioning,
            None,
            Some(&path.provisioning.stages[0]),
            &BankMemory::default(),
            &banks,
            &quests,
        );
        if owns_inventory {
            assert!(matches!(result, Poll::Ready(Ok(ProvisionEvent::Ready))));
            assert_eq!(provisioner.status.phase, ProvisionPhase::Ready);
        } else {
            assert!(result.is_pending());
            assert_eq!(provisioner.status.phase, ProvisionPhase::Scanning);
        }
    }
}
