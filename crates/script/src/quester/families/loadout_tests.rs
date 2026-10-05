use super::*;
use crate::native::{ledger, HostEffect, InteractionReceipt};
use crate::quester::families::tests as fixtures;
use api::quest_progress::EvidenceStamp;
use api::selected::{ClientRevision, Truth};
use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer, ItemView, StatView, WorldTile};
use std::sync::Arc;
use std::task::Poll;

fn with_path_loadout<R>(
    store_rows: Vec<crate::loadouts_store::Loadout>,
    authored_row: serde_json::Value,
    allow_lower_tier: bool,
    f: impl FnOnce(&Arc<dyn StepPlan>, &Arc<dyn PredicatePlan>, &api::game_data::SelectedGameData) -> R,
) -> R {
    let _home = crate::IsolatedEnv::enter("quester-family-header-loadout");
    let mut store = crate::loadouts_store::LoadoutsStore::with_default_path();
    for row in store_rows {
        store.upsert(row);
    }
    store.save().unwrap();
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = api::quest_facts::QuestCatalog::from_identity(data.quest_identity()).unwrap();
    let mut document: serde_json::Value =
        serde_json::from_str(crate::quester::compile::COOK_JSON).unwrap();
    document["quest"]["loadouts"] = serde_json::json!({"melee": authored_row});
    document["roles"][0]["prelude"] = serde_json::json!([{
        "id": "header-loadout",
        "kind": "loadout",
        "version": 1,
        "args": {"loadout": "melee", "allow_lower_tier": allow_lower_tier},
        "skip_if": {
            "Fact": {
                "kind": "loadout_ready",
                "version": 1,
                "args": {"loadout": "melee", "allow_lower_tier": allow_lower_tier}
            }
        },
        "settle": {"All": []}
    }]);
    let document = serde_json::from_value(document).unwrap();
    let path =
        crate::quester::compile::compile_uncached_for_test(&document, &data, &quests).unwrap();
    f(&path.prelude[0].plan, &path.prelude[0].skip_if, &data)
}

fn path_loadout() -> serde_json::Value {
    serde_json::json!({
        "worn": {"hat": "rune_full_helm"},
        "carry": [{"item": "lobster", "qty": 2}]
    })
}

fn tile() -> WorldTile {
    WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    }
}

fn snapshot(
    inventory: Vec<ItemView>,
    equipment: Vec<ItemView>,
    bank: Option<Vec<ItemView>>,
    stats: Vec<StatView>,
) -> GameSnapshot {
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_local_player(fixtures::local_player(tile()));
    snapshot.seed_inventory(inventory, 28);
    snapshot.seed_equipment(equipment);
    snapshot.seed_stats(stats);
    if let Some(bank) = bank {
        snapshot.seed_bank_observation(100, 1, Some(bank), vec![]);
    }
    snapshot
}

fn bank_item(id: i32, name: &str, count: i32) -> ItemView {
    ItemView {
        def: fixtures::def(id, name),
        container: ItemContainer::Bank,
        action_family: ItemActionFamily::Component,
        slot: 0,
        count,
        actions: vec![Some("Withdraw-X".into()), Some("Withdraw-1".into())],
        component_id: 7,
    }
}

fn held_item(id: i32, name: &str, count: i32, container: ItemContainer) -> ItemView {
    ItemView {
        def: fixtures::def(id, name),
        container,
        action_family: ItemActionFamily::Held,
        slot: 0,
        count,
        actions: vec![],
        component_id: 3214,
    }
}

fn with_step<R>(
    snapshot: &GameSnapshot,
    ledger: &mut Option<Box<ledger::Ledger>>,
    now: u64,
    banks: &Arc<api::named_banks::NamedBankFacts>,
    f: impl FnOnce(&mut StepContext<'_, '_>) -> R,
) -> R {
    fixtures::with_tick(snapshot, ledger, now, |tick| {
        let quests = api::quest_facts::QuestCatalog::empty();
        let required_after = tick.cx.evidence();
        let bank = crate::quester::bank_memo::BankMemo::default();
        let choices = crate::quester::choices::QuestChoices::default();
        f(&mut StepContext {
            tick,
            quests: &quests,
            progress: &[],
            required_after,
            bank: &bank,
            banks,
            choices: &choices,
        })
    })
}

fn drive_to_wear(
    plan: &Arc<dyn StepPlan>,
    snapshot: &mut GameSnapshot,
    bank: api::named_banks::NamedBank,
    banks: &Arc<api::named_banks::NamedBankFacts>,
) -> (Vec<i32>, String) {
    let mut ledger_state = None;
    let mut run = with_step(snapshot, &mut ledger_state, 1, banks, |cx| {
        plan.begin(cx)
            .expect("loadout begin must resolve its compiled items")
    });
    let mut held = Vec::<ItemView>::new();
    let mut withdrawn = Vec::new();

    for now in 2..=48 {
        let result = with_step(snapshot, &mut ledger_state, now, banks, |cx| run.poll(cx));
        match result {
            Poll::Ready(Err(error)) => panic!("loadout run failed before wear: {error:?}"),
            Poll::Ready(Ok(_)) => panic!("loadout run completed without issuing wear"),
            Poll::Pending => {}
        }

        let Some(ledger) = ledger_state.as_ref() else {
            continue;
        };
        if let Some(index) = ledger
            .outbox
            .iter()
            .position(|action| matches!(&action.effect, HostEffect::BankPick(_)))
        {
            let action = ledger_state.as_mut().unwrap().outbox.remove(index);
            let authority = action.authority();
            ledger_state.as_mut().unwrap().complete_bank_pick(
                &authority,
                crate::bank::BankPickReceipt {
                    request_id: authority.request_id().get(),
                    evidence: EvidenceStamp {
                        run: authority.run(),
                        tick: now,
                        sequence: now,
                    },
                    selected: crate::bank::SelectedBank {
                        bank_index: 0,
                        access_tile: tile(),
                        kind: crate::bank::PickKind::Reachable,
                        access: Some(Arc::new(crate::bank::BankStandAccess {
                            bank,
                            stand_tile: tile(),
                            kind: crate::bank::AccessKind::Booth,
                            stand_op: 1,
                            name: None,
                            choose: None,
                        })),
                    },
                },
            );
            continue;
        }
        let Some(index) = ledger
            .outbox
            .iter()
            .position(|action| matches!(&action.effect, HostEffect::Interaction(_)))
        else {
            continue;
        };
        let action = ledger_state.as_mut().unwrap().outbox.remove(index);
        let authority = action.authority();
        let HostEffect::Interaction(request) = &action.effect else {
            unreachable!();
        };
        match request {
            crate::shim::InteractReq::Wear { name } => {
                return (withdrawn, name.clone());
            }
            crate::shim::InteractReq::WithdrawX {
                name,
                count,
                bank_item_id,
                ..
            } => {
                withdrawn.push(*bank_item_id);
                ledger_state.as_mut().unwrap().complete_interaction(
                    &authority,
                    InteractionReceipt {
                        request_id: authority.request_id().get(),
                        evidence: EvidenceStamp {
                            run: authority.run(),
                            tick: now,
                            sequence: now,
                        },
                        accepted: true,
                        chat_since: 0,
                    },
                );
                if let Some(held) = held.iter_mut().find(|held| held.def.id == *bank_item_id) {
                    held.count += *count;
                } else {
                    held.push(held_item(
                        *bank_item_id,
                        name,
                        *count,
                        ItemContainer::Inventory,
                    ));
                }
                snapshot.seed_inventory(held.clone(), 28);
            }
            crate::shim::InteractReq::SetNoteMode { .. } => {
                ledger_state.as_mut().unwrap().complete_interaction(
                    &authority,
                    InteractionReceipt {
                        request_id: authority.request_id().get(),
                        evidence: EvidenceStamp {
                            run: authority.run(),
                            tick: now,
                            sequence: now,
                        },
                        accepted: true,
                        chat_since: 0,
                    },
                );
            }
            crate::shim::InteractReq::Close => {
                ledger_state.as_mut().unwrap().complete_interaction(
                    &authority,
                    InteractionReceipt {
                        request_id: authority.request_id().get(),
                        evidence: EvidenceStamp {
                            run: authority.run(),
                            tick: now,
                            sequence: now,
                        },
                        accepted: true,
                        chat_since: 0,
                    },
                );
                snapshot.seed_bank_observation(-1, now, None, vec![]);
            }
            other => panic!("unexpected loadout interaction before wear: {other:?}"),
        }
    }
    panic!("loadout did not reach its equipment request")
}

fn evaluate_ready(plan: &Arc<dyn PredicatePlan>, snapshot: &GameSnapshot) -> Truth {
    let mut ledger_state = None;
    fixtures::with_tick(snapshot, &mut ledger_state, 1, |tick| {
        let quests = api::quest_facts::QuestCatalog::empty();
        let required_after = tick.cx.evidence();
        let bank = crate::quester::bank_memo::BankMemo::default();
        plan.evaluate(&PredicateContext {
            cx: &tick.cx,
            quests: &quests,
            progress: &[],
            required_after,
            chat_since: 0,
            outcome: None,
            bank: &bank,
        })
    })
}

fn bank_facts() -> (
    api::named_banks::NamedBank,
    Arc<api::named_banks::NamedBankFacts>,
) {
    let bank = api::named_banks::NamedBank::new("Alias test bank", tile());
    let facts = Arc::new(api::named_banks::NamedBankFacts::from_banks(vec![bank]));
    (bank, facts)
}

#[test]
fn path_alias_worn_and_carry_queue_the_exact_unnoted_ids() {
    with_path_loadout(vec![], path_loadout(), false, |plan, _, data| {
        let lobster = data.item_by_alias("lobster").unwrap();
        let cert_lobster = data.item_by_alias("cert_lobster").unwrap();
        let helm = data.item_by_alias("rune_full_helm").unwrap();
        let cert_helm = data.item_by_alias("cert_rune_full_helm").unwrap();
        assert!(!lobster.is_certificate());
        assert!(!helm.is_certificate());
        assert!(cert_lobster.is_certificate());
        assert!(cert_helm.is_certificate());

        let (bank, banks) = bank_facts();
        let mut snapshot = snapshot(
            vec![],
            vec![],
            Some(vec![
                bank_item(lobster.id, "Lobster", 10),
                bank_item(cert_lobster.id, "Lobster", 10),
                bank_item(helm.id, "Rune full helm", 10),
                bank_item(cert_helm.id, "Rune full helm", 10),
            ]),
            vec![],
        );
        let (withdrawn, worn) = drive_to_wear(plan, &mut snapshot, bank, &banks);
        assert_eq!(withdrawn, [lobster.id, helm.id]);
        assert_eq!(worn, "Rune full helm");
    });
}

#[test]
fn alias_tier_fallback_resolves_real_candidates_and_keeps_stat_gates() {
    let loadout = serde_json::json!({"worn": {"hat": "rune_full_helm"}, "carry": []});
    with_path_loadout(vec![], loadout, true, |plan, _, data| {
        let adamant = data.item_by_alias("adamant_full_helm").unwrap();
        let defence_40 = vec![StatView {
            index: 1,
            name: "defence".into(),
            effective: 40,
            base: 40,
            xp: 0,
            used: true,
        }];
        let (bank, banks) = bank_facts();
        let mut strong_snapshot = snapshot(
            vec![],
            vec![],
            Some(vec![bank_item(adamant.id, "Adamant full helm", 5)]),
            defence_40,
        );
        let (withdrawn, worn) = drive_to_wear(plan, &mut strong_snapshot, bank, &banks);
        assert_eq!(withdrawn, [adamant.id]);
        assert_eq!(worn, "Adamant full helm");

        let weak_snapshot = snapshot(vec![], vec![], None, vec![]);
        let no_banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let mut ledger_state = None;
        assert!(matches!(
            with_step(&weak_snapshot, &mut ledger_state, 1, &no_banks, |cx| plan.begin(cx)),
            Err(ActionError::Unavailable(reason))
                if reason.as_ref() == "no stat/quest-legal loadout tier"
        ));

        let names = vec!["Dragon longsword".to_owned(), "Rune sword".to_owned()];
        let quest_gated = crate::quester::loadouts::tier_candidates(
            "righthand",
            "Dragon longsword",
            &names,
            crate::quester::loadouts::TierFacts {
                attack: 60,
                ..Default::default()
            },
            &data
                .equipment_names()
                .expect("selected melee family")
                .melee_weapons,
        );
        assert_eq!(quest_gated, ["Rune sword"]);
    });
}

#[test]
fn loadout_ready_uses_alias_ids_and_excludes_same_display_certificates() {
    with_path_loadout(vec![], path_loadout(), true, |_, ready, data| {
        let lobster = data.item_by_alias("lobster").unwrap();
        let cert_lobster = data.item_by_alias("cert_lobster").unwrap();
        let adamant = data.item_by_alias("adamant_full_helm").unwrap();
        let cert_helm = data.item_by_alias("cert_rune_full_helm").unwrap();

        let base_snapshot = snapshot(
            vec![held_item(
                lobster.id,
                "Lobster",
                2,
                ItemContainer::Inventory,
            )],
            vec![held_item(
                adamant.id,
                "Adamant full helm",
                1,
                ItemContainer::Equipment,
            )],
            None,
            vec![],
        );
        assert_eq!(evaluate_ready(ready, &base_snapshot), Truth::True);

        let cert_helm_snapshot = snapshot(
            vec![held_item(
                lobster.id,
                "Lobster",
                2,
                ItemContainer::Inventory,
            )],
            vec![held_item(
                cert_helm.id,
                "Rune full helm",
                1,
                ItemContainer::Equipment,
            )],
            None,
            vec![],
        );
        assert_eq!(evaluate_ready(ready, &cert_helm_snapshot), Truth::False);

        let cert_lobster_snapshot = snapshot(
            vec![held_item(
                cert_lobster.id,
                "Lobster",
                2,
                ItemContainer::Inventory,
            )],
            vec![held_item(
                adamant.id,
                "Adamant full helm",
                1,
                ItemContainer::Equipment,
            )],
            None,
            vec![],
        );
        assert_eq!(evaluate_ready(ready, &cert_lobster_snapshot), Truth::False);
    });
}

#[test]
fn operator_display_name_override_still_selects_unnoted_items() {
    let store = crate::loadouts_store::Loadout::new("cook/melee")
        .with_slot("hat", "Rune full helm")
        .with_carry("Lobster", 2);
    with_path_loadout(vec![store], path_loadout(), false, |plan, _, data| {
        let lobster = data.item_by_alias("lobster").unwrap();
        let cert_lobster = data.item_by_alias("cert_lobster").unwrap();
        let helm = data.item_by_alias("rune_full_helm").unwrap();
        let (bank, banks) = bank_facts();
        let mut snapshot = snapshot(
            vec![],
            vec![],
            Some(vec![
                bank_item(lobster.id, "Lobster", 10),
                bank_item(cert_lobster.id, "Lobster", 10),
                bank_item(helm.id, "Rune full helm", 10),
            ]),
            vec![],
        );
        let (withdrawn, worn) = drive_to_wear(plan, &mut snapshot, bank, &banks);
        assert_eq!(withdrawn, [lobster.id, helm.id]);
        assert_eq!(worn, "Rune full helm");
    });
}
