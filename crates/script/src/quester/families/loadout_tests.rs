use super::*;
use crate::native::{ledger, HostEffect, InteractionReceipt};
use crate::quester::families::tests as fixtures;
use api::bank_memory::{BankMemory, Origin};
use api::quest_progress::EvidenceStamp;
use api::selected::{ClientRevision, Truth};
use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer, ItemView, StatView, WorldTile};
use std::sync::Arc;
use std::task::Poll;

fn with_path_loadout<R>(
    store_rows: Vec<api::loadout::Loadout>,
    authored_row: serde_json::Value,
    allow_lower_tier: bool,
    f: impl FnOnce(&Arc<dyn StepPlan>, &Arc<dyn PredicatePlan>, &api::game_data::SelectedGameData) -> R,
) -> R {
    with_path_loadout_args(
        store_rows,
        authored_row,
        serde_json::json!({"allow_lower_tier": allow_lower_tier}),
        f,
    )
}

/// `args` are the step's authored `loadout` arguments besides `loadout`
/// itself; the `loadout_ready` skip gets the same ones.
fn with_path_loadout_args<R>(
    store_rows: Vec<api::loadout::Loadout>,
    authored_row: serde_json::Value,
    mut args: serde_json::Value,
    f: impl FnOnce(&Arc<dyn StepPlan>, &Arc<dyn PredicatePlan>, &api::game_data::SelectedGameData) -> R,
) -> R {
    let data_rows: Arc<[api::loadout::Loadout]> = Arc::from(store_rows);
    let operator = crate::local_state::OperatorLoadouts::from_rows(
        data_rows,
        api::local_state::StoreVersion::INITIAL,
    );
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = api::quest_facts::QuestCatalog::from_identity(data.quest_identity()).unwrap();
    let mut document: serde_json::Value =
        serde_json::from_str(crate::quester::compile::COOK_JSON).unwrap();
    document["quest"]["loadouts"] = serde_json::json!({"melee": authored_row});
    args["loadout"] = serde_json::json!("melee");
    document["roles"][0]["prelude"] = serde_json::json!([{
        "id": "header-loadout",
        "kind": "loadout",
        "version": 1,
        "args": args,
        "skip_if": {
            "Fact": {
                "kind": "loadout_ready",
                "version": 1,
                "args": args
            }
        },
        "settle": {"All": []}
    }]);
    let document = serde_json::from_value(document).unwrap();
    let path = crate::quester::compile::compile_uncached_for_test_with_loadouts(
        &document, &data, &quests, &operator,
    )
    .unwrap();
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
        let choices = crate::quester::choices::QuestChoices::default();
        f(&mut StepContext {
            tick,
            quests: &quests,
            progress: &[],
            required_after,
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
                    outcome: crate::bank::BankPickOutcome::Selected(crate::bank::SelectedBank {
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
                    }),
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
            crate::interact::InteractReq::Wear { name } => {
                return (withdrawn, name.clone());
            }
            crate::interact::InteractReq::WithdrawX {
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
            crate::interact::InteractReq::SetNoteMode { .. } => {
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
            crate::interact::InteractReq::Close => {
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
        plan.evaluate(&PredicateContext {
            cx: &tick.cx,
            pairs: tick.pairs,
            quests: &quests,
            progress: &[],
            required_after,
            chat_since: 0,
            outcome: None,
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
fn unobserved_snapshot() -> GameSnapshot {
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot
}

fn begin_unobserved_loadout(
    plan: &Arc<dyn StepPlan>,
    snapshot: &GameSnapshot,
    ledger_state: &mut Option<Box<ledger::Ledger>>,
    banks: &Arc<api::named_banks::NamedBankFacts>,
) -> Box<dyn StepRun> {
    with_step(snapshot, ledger_state, 1, banks, |cx| {
        plan.begin(cx).expect("loadout wait should begin")
    })
}

#[test]
fn loadout_observation_wait_exposes_borrowed_stable_detail() {
    if !api::test_support::game_data_available() {
        return;
    }
    with_path_loadout(vec![], path_loadout(), false, |plan, _, _| {
        let snapshot = unobserved_snapshot();
        let mut ledger_state = None;
        let (_, banks) = bank_facts();
        let run = begin_unobserved_loadout(plan, &snapshot, &mut ledger_state, &banks);
        let first = run.waiting_for().expect("observation wait detail");
        let second = run.waiting_for().expect("stable observation wait detail");
        assert_eq!(first.0, "Loadout observation");
        assert_eq!(
            first.1.as_ref(),
            "Waiting for inventory/equipment observation"
        );
        let other = begin_unobserved_loadout(plan, &snapshot, &mut ledger_state, &banks);
        assert!(std::ptr::eq(first.1, other.waiting_for().unwrap().1));
        assert!(std::ptr::eq(first.1.as_ptr(), second.1.as_ptr()));
    });
}

#[test]
fn loadout_observation_wait_times_out_without_both_observations() {
    if !api::test_support::game_data_available() {
        return;
    }
    with_path_loadout(vec![], path_loadout(), false, |plan, _, _| {
        let (_, banks) = bank_facts();
        let partial_inventory = {
            let mut snapshot = GameSnapshot::new();
            snapshot.seed_ingame(2);
            snapshot.seed_inventory(vec![], 28);
            snapshot
        };
        let partial_equipment = {
            let mut snapshot = GameSnapshot::new();
            snapshot.seed_ingame(2);
            snapshot.seed_equipment(vec![]);
            snapshot
        };
        for snapshot in [unobserved_snapshot(), partial_inventory, partial_equipment] {
            let mut ledger_state = None;
            let mut run = begin_unobserved_loadout(plan, &snapshot, &mut ledger_state, &banks);
            assert!(matches!(
                with_step(&snapshot, &mut ledger_state, 50, &banks, |cx| run.poll(cx)),
                Poll::Pending
            ));
            assert!(matches!(
                with_step(&snapshot, &mut ledger_state, 51, &banks, |cx| run.poll(cx)),
                Poll::Ready(Err(ActionError::NeedsEvidence(gates))) if gates.is_empty()
            ));
        }
    });
}

#[test]
fn loadout_observation_at_deadline_wins_and_cancellation_dispatches_nothing() {
    if !api::test_support::game_data_available() {
        return;
    }
    with_path_loadout(
        vec![],
        serde_json::json!({"worn":{}, "carry":[]}),
        false,
        |plan, _, _| {
            let (_, banks) = bank_facts();
            let missing = unobserved_snapshot();
            let mut ledger_state = None;
            let mut run = begin_unobserved_loadout(plan, &missing, &mut ledger_state, &banks);
            let mut actions = crate::native::NativeActions { _private: () };
            run.cancel(&mut actions);
            assert!(ledger_state
                .as_ref()
                .is_none_or(|ledger| ledger.outbox.is_empty()));

            let mut readiness_ledger = None;
            let mut ready_run =
                begin_unobserved_loadout(plan, &missing, &mut readiness_ledger, &banks);
            let observed = snapshot(vec![], vec![], None, vec![]);
            let result = with_step(&observed, &mut readiness_ledger, 51, &banks, |cx| {
                ready_run.poll(cx)
            });
            assert!(matches!(result, Poll::Ready(Ok(_))));
        },
    );
}

#[test]
fn path_alias_worn_and_carry_queue_the_exact_unnoted_ids() {
    if !api::test_support::game_data_available() {
        return;
    }
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
    if !api::test_support::game_data_available() {
        return;
    }
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
fn real_loadout_candidates_match_canonical_first_with_fewer_name_lookups() {
    if !api::test_support::game_data_available() {
        return;
    }
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let helm = selected.item_by_alias("rune_full_helm").unwrap();
    let lobster = selected.item_by_alias("lobster").unwrap();
    let helm_name = helm.name.as_deref().unwrap();
    let mut canonical_first = Vec::new();
    let mut relevance_first = Vec::new();
    let mut all_named = 0;
    let mut relevant_named = 0;

    for candidate in selected.items() {
        if candidate.is_certificate() {
            continue;
        }
        let Some(name) = candidate.name.as_deref() else {
            continue;
        };
        all_named += 1;
        // This is the real hat + carry loadout used above, including its
        // lower-metal tier fallback. Compare the old and new lookup order over
        // every selected item, not a hand-picked table of names.
        let relevant = candidate.id == lobster.id
            || candidate.id == helm.id
            || crate::melee_weapons::same_or_lower_metal_item(name, helm_name);
        if selected
            .resolve_item_name(name)
            .is_some_and(|item| item.id == candidate.id)
            && relevant
        {
            canonical_first.push(candidate.id);
        }
        if relevant {
            relevant_named += 1;
            if selected
                .resolve_item_name(name)
                .is_some_and(|item| item.id == candidate.id)
            {
                relevance_first.push(candidate.id);
            }
        }
    }
    assert!(canonical_first.contains(&helm.id));
    assert!(canonical_first.contains(&lobster.id));
    assert_eq!(canonical_first, relevance_first);
    assert!(
        relevant_named * 8 < all_named,
        "irrelevant selected names must not consume canonical lookups"
    );
}

#[test]
fn loadout_ready_uses_alias_ids_and_excludes_same_display_certificates() {
    if !api::test_support::game_data_available() {
        return;
    }
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
    if !api::test_support::game_data_available() {
        return;
    }
    let store = api::loadout::Loadout::new("cook/melee")
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

fn with_step_bank<R>(
    snapshot: &GameSnapshot,
    bank: Option<&BankMemory>,
    ledger: &mut Option<Box<ledger::Ledger>>,
    now: u64,
    banks: &Arc<api::named_banks::NamedBankFacts>,
    f: impl FnOnce(&mut StepContext<'_, '_>) -> R,
) -> R {
    fixtures::with_tick_bank(snapshot, bank, ledger, now, |tick| {
        let quests = api::quest_facts::QuestCatalog::empty();
        let required_after = tick.cx.evidence();
        let choices = crate::quester::choices::QuestChoices::default();
        f(&mut StepContext {
            tick,
            quests: &quests,
            progress: &[],
            required_after,
            banks,
            choices: &choices,
        })
    })
}

fn defence_40() -> Vec<StatView> {
    vec![StatView {
        index: 1,
        name: "defence".into(),
        effective: 40,
        base: 40,
        xp: 0,
        used: true,
    }]
}

/// How the fixture bank answers a loadout trip.
struct FixtureBank {
    bank: api::named_banks::NamedBank,
    /// Rows the bank shows once the run opens it (a closed snapshot bank);
    /// `None` forbids any selection or open: the run must reuse the open
    /// bank (design-bank-snapshot §4 F3).
    opens_with: Option<Vec<ItemView>>,
}

/// Drive a loadout run with the account's memory attached, tracking the
/// snapshot's bank into it before every tick the way the host does.
/// Returns the bank item ids whose withdraw was clicked and how the run
/// ended: `Ok(Some(name))` at its first `Wear`, `Ok(None)` when it
/// completed without one. An `Unequip` moves the worn row into the pack.
fn drive_loadout(
    plan: &Arc<dyn StepPlan>,
    snapshot: &mut GameSnapshot,
    memory: &mut BankMemory,
    fixture: &FixtureBank,
    banks: &Arc<api::named_banks::NamedBankFacts>,
) -> (Vec<i32>, Result<Option<String>, ActionError>) {
    let mut ledger_state = None;
    memory.track(snapshot, 1);
    let mut run = with_step_bank(snapshot, Some(memory), &mut ledger_state, 1, banks, |cx| {
        plan.begin(cx)
            .expect("loadout begin must resolve its compiled items")
    });
    let mut withdrawn = Vec::new();
    for now in 2..=48 {
        memory.track(snapshot, now);
        let result = with_step_bank(
            snapshot,
            Some(memory),
            &mut ledger_state,
            now,
            banks,
            |cx| run.poll(cx),
        );
        match result {
            Poll::Ready(Err(error)) => return (withdrawn, Err(error)),
            Poll::Ready(Ok(_)) => return (withdrawn, Ok(None)),
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
            assert!(
                fixture.opens_with.is_some(),
                "an open bank is reused: no bank selection"
            );
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
                    outcome: crate::bank::BankPickOutcome::Selected(crate::bank::SelectedBank {
                        bank_index: 0,
                        access_tile: tile(),
                        kind: crate::bank::PickKind::Reachable,
                        access: Some(Arc::new(crate::bank::BankStandAccess {
                            bank: fixture.bank,
                            stand_tile: tile(),
                            kind: crate::bank::AccessKind::Booth,
                            stand_op: 1,
                            name: None,
                            choose: None,
                        })),
                    }),
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
        let accept = |ledger: &mut Option<Box<ledger::Ledger>>| {
            ledger.as_mut().unwrap().complete_interaction(
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
        };
        match request {
            crate::interact::InteractReq::Wear { name } => {
                return (withdrawn, Ok(Some(name.clone())));
            }
            crate::interact::InteractReq::Unequip { name } => {
                accept(&mut ledger_state);
                let mut equipment = snapshot.equipment().to_vec();
                let index = equipment
                    .iter()
                    .position(|row| row.def.name.as_deref() == Some(name))
                    .expect("an Unequip names a worn item");
                let removed = equipment.remove(index);
                let mut inventory = snapshot.inventory().to_vec();
                inventory.push(held_item(
                    removed.def.id,
                    name,
                    removed.count,
                    ItemContainer::Inventory,
                ));
                snapshot.seed_equipment(equipment);
                snapshot.seed_inventory(inventory, 28);
            }
            crate::interact::InteractReq::OpenStand { .. } => {
                let rows = fixture
                    .opens_with
                    .clone()
                    .expect("an open bank is reused: no open");
                accept(&mut ledger_state);
                snapshot.seed_bank_observation(100, now, Some(rows), vec![]);
            }
            crate::interact::InteractReq::WithdrawX {
                name,
                count,
                bank_item_id,
                ..
            } => {
                withdrawn.push(*bank_item_id);
                accept(&mut ledger_state);
                let mut inventory = snapshot.inventory().to_vec();
                if let Some(held) = inventory.iter_mut().find(|row| row.def.id == *bank_item_id) {
                    held.count += *count;
                } else {
                    inventory.push(held_item(
                        *bank_item_id,
                        name,
                        *count,
                        ItemContainer::Inventory,
                    ));
                }
                snapshot.seed_inventory(inventory, 28);
            }
            crate::interact::InteractReq::SetNoteMode { .. } => accept(&mut ledger_state),
            crate::interact::InteractReq::Close => {
                accept(&mut ledger_state);
                snapshot.seed_bank_observation(-1, now, None, vec![]);
            }
            other => panic!("unexpected loadout interaction: {other:?}"),
        }
    }
    panic!("loadout did not settle")
}

/// design-bank-snapshot §6 bug 15: a legal tier already worn is the slot
/// satisfied — no bank trip, not even with an unknown bank.
#[test]
fn worn_legal_tier_takes_no_bank_action() {
    if !api::test_support::game_data_available() {
        return;
    }
    let loadout = serde_json::json!({"worn": {"hat": "rune_full_helm"}, "carry": []});
    with_path_loadout(vec![], loadout, true, |plan, _, data| {
        let adamant = data.item_by_alias("adamant_full_helm").unwrap();
        let (_, banks) = bank_facts();
        let snapshot = snapshot(
            vec![],
            vec![held_item(
                adamant.id,
                "Adamant full helm",
                1,
                ItemContainer::Equipment,
            )],
            None,
            defence_40(),
        );
        let mut ledger_state = None;
        let mut run = with_step(&snapshot, &mut ledger_state, 1, &banks, |cx| {
            plan.begin(cx).expect("loadout begin")
        });
        assert!(matches!(
            with_step(&snapshot, &mut ledger_state, 2, &banks, |cx| run.poll(cx)),
            Poll::Ready(Ok(_))
        ));
        assert!(
            ledger_state
                .as_ref()
                .is_none_or(|ledger| ledger.outbox.is_empty()),
            "a worn legal tier issues no bank or equipment request"
        );
    });
}

/// design-bank-snapshot §4 F4: with a known bank the strongest *banked*
/// tier is a plain `Withdraw`, not a blind `WithdrawAny`. A hint claiming a
/// tier the open bank lacks therefore costs exactly one failed trip (D4),
/// after which the memory is `Session` and the retry withdraws what the
/// bank really holds.
#[test]
fn known_bank_withdraws_the_strongest_banked_tier_not_withdraw_any() {
    if !api::test_support::game_data_available() {
        return;
    }
    let loadout = serde_json::json!({"worn": {"hat": "rune_full_helm"}, "carry": []});
    with_path_loadout(vec![], loadout, true, |plan, _, data| {
        let adamant = data.item_by_alias("adamant_full_helm").unwrap();
        let steel = data.item_by_alias("steel_full_helm").unwrap();
        let (bank, banks) = bank_facts();
        // The bank is closed when the step begins; the trip selects the
        // booth at the player's tile and opens it on a steel helm only.
        let mut snapshot = snapshot(vec![], vec![], None, defence_40());
        snapshot.seed_locs(vec![api::snapshot::LocView {
            id: 2213,
            name: Some("Bank booth".into()),
            actions: vec![Some("Use-quickly".into())],
            tile: tile(),
            distance: 0,
            typecode: 0,
            info: 0,
            description: None,
            layer: api::snapshot::LocLayer::GroundDecoration,
            shape: 0,
            angle: 0,
            width: 1,
            length: 1,
            footprint_width: 1,
            footprint_length: 1,
            block_walk: false,
            block_range: false,
            active: true,
            animation: -1,
            map_function: -1,
            map_scene: -1,
            force_approach: 0,
        }]);
        let fixture = FixtureBank {
            bank,
            opens_with: Some(vec![bank_item(steel.id, "Steel full helm", 1)]),
        };
        // Stale positive: the hint banks an adamant helm the bank lacks.
        let mut memory = BankMemory::seeded(&[(adamant.id, 1)], Origin::Hint);
        let (withdrawn, end) = drive_loadout(plan, &mut snapshot, &mut memory, &fixture, &banks);
        assert!(
            withdrawn.is_empty(),
            "no weaker tier is taken behind the hint's back"
        );
        assert!(
            matches!(&end, Err(ActionError::Failed(reason)) if reason.as_ref() == "bank lacks requested item"),
            "a plain Withdraw of the hinted tier fails at the open bank: {end:?}"
        );
        assert_eq!(memory.origin(), Origin::Session);
        assert_eq!(memory.count(adamant.id), Some(0));
        assert_eq!(memory.count(steel.id), Some(1));

        // The retry plans from `Session` rows at the still-open bank: the
        // strongest banked tier, with no second selection or open.
        let fixture = FixtureBank {
            bank,
            opens_with: None,
        };
        let (withdrawn, end) = drive_loadout(plan, &mut snapshot, &mut memory, &fixture, &banks);
        assert_eq!(withdrawn, [steel.id]);
        assert_eq!(end.unwrap().as_deref(), Some("Steel full helm"));
    });
}

/// design-bank-snapshot §2.4 (D4): a `Session` bank with no legal tier and
/// nothing carried refuses the step in place — no selection, no walk.
#[test]
fn session_bank_with_no_legal_tier_is_refused_in_place() {
    if !api::test_support::game_data_available() {
        return;
    }
    let loadout = serde_json::json!({"worn": {"hat": "rune_full_helm"}, "carry": []});
    with_path_loadout(vec![], loadout, true, |plan, _, _| {
        let (_, banks) = bank_facts();
        let snapshot = snapshot(vec![], vec![], None, defence_40());
        let memory = BankMemory::seeded(&[], Origin::Session);
        let mut ledger_state = None;
        let mut run = with_step_bank(
            &snapshot,
            Some(&memory),
            &mut ledger_state,
            1,
            &banks,
            |cx| plan.begin(cx).expect("loadout begin"),
        );
        let result = with_step_bank(
            &snapshot,
            Some(&memory),
            &mut ledger_state,
            2,
            &banks,
            |cx| run.poll(cx),
        );
        let Poll::Ready(Err(ActionError::Blocked(reason))) = result else {
            panic!("a session bank with no legal tier must refuse in place");
        };
        assert!(
            reason.starts_with("no permitted tier carried or banked: "),
            "{reason}"
        );
        assert!(ledger_state
            .as_ref()
            .is_none_or(|ledger| ledger.outbox.is_empty()));
    });
}

/// design-bank-snapshot §4 F3: a loadout step that follows provisioning at
/// an open bank reuses it — the first effect is the withdraw itself.
#[test]
fn loadout_at_an_open_bank_issues_no_select_walk_or_open() {
    if !api::test_support::game_data_available() {
        return;
    }
    with_path_loadout(vec![], path_loadout(), false, |plan, _, data| {
        let lobster = data.item_by_alias("lobster").unwrap();
        let helm = data.item_by_alias("rune_full_helm").unwrap();
        let (bank, banks) = bank_facts();
        let mut snapshot = snapshot(
            vec![],
            vec![],
            Some(vec![
                bank_item(lobster.id, "Lobster", 10),
                bank_item(helm.id, "Rune full helm", 1),
            ]),
            vec![],
        );
        let fixture = FixtureBank {
            bank,
            opens_with: None,
        };
        let mut memory = BankMemory::default();
        let (withdrawn, end) = drive_loadout(plan, &mut snapshot, &mut memory, &fixture, &banks);
        assert_eq!(withdrawn, [lobster.id, helm.id]);
        assert_eq!(end.unwrap().as_deref(), Some("Rune full helm"));
        assert_eq!(
            memory.origin(),
            Origin::Session,
            "the open bank was observed"
        );
    });
}

/// A legal tier that is held but not worn still needs its `Wear`. The
/// missing carry requires an ordinary bank trip; the Wear is sent without
/// a separate Close, so the engine closes the bank as it equips the helm.
#[test]
fn held_legal_tier_is_worn_after_carry_trip_without_a_separate_close() {
    if !api::test_support::game_data_available() {
        return;
    }
    with_path_loadout(vec![], path_loadout(), true, |plan, _, data| {
        let adamant = data.item_by_alias("adamant_full_helm").unwrap();
        let lobster = data.item_by_alias("lobster").unwrap();
        let (bank, banks) = bank_facts();
        let mut snapshot = snapshot(
            vec![held_item(
                adamant.id,
                "Adamant full helm",
                1,
                ItemContainer::Inventory,
            )],
            vec![],
            Some(vec![bank_item(lobster.id, "Lobster", 10)]),
            defence_40(),
        );
        let fixture = FixtureBank {
            bank,
            opens_with: None,
        };
        let mut memory = BankMemory::default();
        let (withdrawn, end) = drive_loadout(plan, &mut snapshot, &mut memory, &fixture, &banks);
        assert_eq!(withdrawn, [lobster.id], "only the carry row is withdrawn");
        assert_eq!(end.unwrap().as_deref(), Some("Adamant full helm"));
        assert!(
            snapshot.bank_component_id() >= 0,
            "the held-tier Wear is sent before any separate bank Close"
        );
    });
}

/// Everything is held at an incoming open bank: no trip is needed, so
/// Quester sends the Wear without a separate Close.
#[test]
fn fully_held_loadout_wears_with_open_bank() {
    if !api::test_support::game_data_available() {
        return;
    }
    with_path_loadout(vec![], path_loadout(), true, |plan, _, data| {
        let adamant = data.item_by_alias("adamant_full_helm").unwrap();
        let lobster = data.item_by_alias("lobster").unwrap();
        let (bank, banks) = bank_facts();
        let mut snapshot = snapshot(
            vec![
                held_item(adamant.id, "Adamant full helm", 1, ItemContainer::Inventory),
                held_item(lobster.id, "Lobster", 2, ItemContainer::Inventory),
            ],
            vec![],
            Some(vec![]),
            defence_40(),
        );
        let fixture = FixtureBank {
            bank,
            opens_with: None,
        };
        let mut memory = BankMemory::default();
        let (withdrawn, end) = drive_loadout(plan, &mut snapshot, &mut memory, &fixture, &banks);
        assert!(withdrawn.is_empty(), "nothing to withdraw");
        assert_eq!(end.unwrap().as_deref(), Some("Adamant full helm"));
        assert!(
            snapshot.bank_component_id() >= 0,
            "the Wear leaves bank closure to the engine"
        );
    });
}

/// An exclusive loadout removes extra worn items after the bank trip. It
/// sends Unequip with the bank open; 289 closes it on that interaction.
#[test]
fn exclusive_strip_follows_the_trip_without_a_separate_close() {
    if !api::test_support::game_data_available() {
        return;
    }
    let loadout = serde_json::json!({"worn": {"hat": "rune_full_helm"}, "carry": []});
    let args = serde_json::json!({"exclusive": true});
    with_path_loadout_args(vec![], loadout, args, |plan, _, data| {
        let helm = data.item_by_alias("rune_full_helm").unwrap();
        let boots = data.item_by_alias("desert_boots").unwrap();
        let (bank, banks) = bank_facts();
        let mut snapshot = snapshot(
            vec![],
            vec![held_item(
                boots.id,
                "Desert boots",
                1,
                ItemContainer::Equipment,
            )],
            Some(vec![bank_item(helm.id, "Rune full helm", 1)]),
            vec![],
        );
        let fixture = FixtureBank {
            bank,
            opens_with: None,
        };
        let mut memory = BankMemory::default();
        let (withdrawn, end) = drive_loadout(plan, &mut snapshot, &mut memory, &fixture, &banks);
        assert_eq!(withdrawn, [helm.id]);
        assert_eq!(end.unwrap().as_deref(), Some("Rune full helm"));
        assert!(
            snapshot.bank_component_id() >= 0,
            "the extra-item removal and Wear need no separate bank Close"
        );
        assert!(
            snapshot.equipment().is_empty()
                && snapshot
                    .inventory()
                    .iter()
                    .any(|row| row.def.id == boots.id),
            "the extra boots were removed into the pack before the Wear"
        );
    });
}

#[test]
fn survival_loadout_wears_each_piece_before_withdrawing_the_next() {
    let items = [
        BankItem {
            id: 3001,
            name: Arc::from("Survival helm"),
        },
        BankItem {
            id: 3002,
            name: Arc::from("Survival body"),
        },
        BankItem {
            id: 3003,
            name: Arc::from("Survival boots"),
        },
    ];
    let (bank, banks) = bank_facts();
    let mut bank_rows = items
        .iter()
        .map(|item| bank_item(item.id, item.name.as_ref(), 1))
        .collect::<Vec<_>>();
    let inventory = (0..27)
        .map(|slot| held_item(2000 + slot, "Pack filler", 1, ItemContainer::Inventory))
        .collect();
    let mut snapshot = snapshot(inventory, Vec::new(), Some(bank_rows.clone()), Vec::new());
    let bank_counts = items.iter().map(|item| (item.id, 1)).collect::<Vec<_>>();
    let mut memory = BankMemory::seeded(&bank_counts, Origin::Session);
    memory.track(&snapshot, 1);

    let mut ledger_state = None;
    let mut run = with_step_bank(
        &snapshot,
        Some(&memory),
        &mut ledger_state,
        1,
        &banks,
        |cx| {
            begin_survival_loadout(&items, Some(bank), false, Arc::from([2000]), false, cx)
                .expect("survival loadout begins")
        },
    );
    let mut events = Vec::new();
    let mut withdrawals = 0;
    let mut completed = false;

    for now in 2..=96 {
        memory.track(&snapshot, now);
        match with_step_bank(
            &snapshot,
            Some(&memory),
            &mut ledger_state,
            now,
            &banks,
            |cx| run.poll(cx),
        ) {
            Poll::Ready(Ok(_)) => {
                completed = true;
                break;
            }
            Poll::Ready(Err(error)) => panic!("survival loadout failed: {error:?}"),
            Poll::Pending => {}
        }

        while let Some(action) = ledger_state
            .as_mut()
            .and_then(|ledger| (!ledger.outbox.is_empty()).then(|| ledger.outbox.remove(0)))
        {
            let authority = action.authority();
            match &action.effect {
                HostEffect::Interaction(crate::interact::InteractReq::WithdrawX {
                    name,
                    count,
                    bank_item_id,
                    ..
                }) => {
                    let expected = items
                        .get(withdrawals)
                        .expect("one withdrawal per survival item");
                    assert_eq!(*bank_item_id, expected.id);
                    assert_eq!(name.as_str(), expected.name.as_ref());
                    assert_eq!(*count, 1);
                    assert_eq!(
                        snapshot
                            .inventory()
                            .iter()
                            .filter(|row| row.count > 0)
                            .count(),
                        27,
                        "each next piece starts with one available pack slot"
                    );
                    let worn = snapshot
                        .equipment()
                        .iter()
                        .filter(|row| row.count > 0)
                        .map(|row| row.def.id)
                        .collect::<Vec<_>>();
                    assert_eq!(
                        worn,
                        items[..withdrawals]
                            .iter()
                            .map(|item| item.id)
                            .collect::<Vec<_>>(),
                        "the prior piece must be observed worn before the next withdrawal"
                    );
                    events.push(('w', expected.id));
                    withdrawals += 1;
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
                    let mut held = snapshot.inventory().to_vec();
                    held.push(held_item(
                        expected.id,
                        name,
                        *count,
                        ItemContainer::Inventory,
                    ));
                    snapshot.seed_inventory(held, 28);
                    let bank_row = bank_rows
                        .iter_mut()
                        .find(|row| row.def.id == expected.id)
                        .expect("withdrawn item remains in the bank table");
                    bank_row.count -= *count;
                    snapshot.seed_bank_observation(100, now, Some(bank_rows.clone()), vec![]);
                }
                HostEffect::Interaction(crate::interact::InteractReq::Wear { name }) => {
                    let expected = items
                        .get(events.len() / 2)
                        .expect("wear follows its selected withdrawal");
                    assert_eq!(name.as_str(), expected.name.as_ref());
                    assert!(
                        !snapshot
                            .equipment()
                            .iter()
                            .any(|row| row.count > 0 && row.def.id == expected.id),
                        "the Wear request precedes its equipment observation"
                    );
                    events.push(('e', expected.id));
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
                    let mut held = snapshot.inventory().to_vec();
                    let index = held
                        .iter()
                        .position(|row| row.def.id == expected.id)
                        .expect("worn piece is held before Wear");
                    let equipped = held.remove(index);
                    let mut worn = snapshot.equipment().to_vec();
                    worn.push(held_item(
                        expected.id,
                        name,
                        equipped.count,
                        ItemContainer::Equipment,
                    ));
                    snapshot.seed_inventory(held, 28);
                    snapshot.seed_equipment(worn);
                }
                HostEffect::Interaction(crate::interact::InteractReq::SetNoteMode { .. }) => {
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
                _ => panic!("unexpected survival loadout effect"),
            }
        }
    }

    assert!(completed, "survival loadout did not finish");
    assert_eq!(withdrawals, items.len());
    assert_eq!(
        events,
        items
            .iter()
            .flat_map(|item| [('w', item.id), ('e', item.id)])
            .collect::<Vec<_>>()
    );
    assert_eq!(snapshot.inventory().len(), 27);
    assert_eq!(
        snapshot
            .equipment()
            .iter()
            .filter(|row| row.count > 0)
            .map(|row| row.def.id)
            .collect::<Vec<_>>(),
        items.iter().map(|item| item.id).collect::<Vec<_>>()
    );
    assert!(
        items
            .iter()
            .all(|item| !snapshot.inventory().iter().any(|row| row.def.id == item.id)),
        "worn survival gear does not occupy pack slots"
    );
}

#[test]
fn survival_loadout_skips_a_session_known_absent_item() {
    let item = BankItem {
        id: 3004,
        name: Arc::from("Absent survival shield"),
    };
    let (bank, banks) = bank_facts();
    let snapshot = snapshot(Vec::new(), Vec::new(), Some(Vec::new()), Vec::new());
    let memory = BankMemory::seeded(&[], Origin::Session);
    let mut ledger_state = None;
    let mut run = with_step_bank(
        &snapshot,
        Some(&memory),
        &mut ledger_state,
        1,
        &banks,
        |cx| {
            begin_survival_loadout(
                std::slice::from_ref(&item),
                Some(bank),
                false,
                Arc::from([]),
                false,
                cx,
            )
            .expect("survival loadout begins")
        },
    );

    assert!(matches!(
        with_step_bank(
            &snapshot,
            Some(&memory),
            &mut ledger_state,
            2,
            &banks,
            |cx| { run.poll(cx) }
        ),
        Poll::Ready(Ok(_))
    ));
    assert!(
        ledger_state
            .as_ref()
            .is_none_or(|ledger| ledger.outbox.is_empty()),
        "a session-known absent item is skipped without a bank action"
    );
}

#[test]
fn survival_loadout_rechecks_managed_worn_items_before_equipping() {
    if !api::test_support::game_data_available() {
        return;
    }
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    for (wanted, protected, slot) in [
        ("bronze_full_helm", "iron_full_helm", 0),
        ("bronze_2h_sword", "bronze_kiteshield", 5),
    ] {
        for protect_worn in [false, true] {
            let wanted = selected.item_by_alias(wanted).unwrap();
            let protected = selected.item_by_alias(protected).unwrap();
            let item = BankItem {
                id: wanted.id,
                name: Arc::from(wanted.name.as_deref().unwrap()),
            };
            let (bank, banks) = bank_facts();
            let mut snapshot = snapshot(
                vec![held_item(item.id, &item.name, 1, ItemContainer::Inventory)],
                Vec::new(),
                None,
                Vec::new(),
            );
            let mut ledger = None;
            let mut run = with_step(&snapshot, &mut ledger, 1, &banks, |cx| {
                begin_survival_loadout(
                    std::slice::from_ref(&item),
                    Some(bank),
                    false,
                    Arc::from([protected.id]),
                    protect_worn,
                    cx,
                )
                .unwrap()
            });
            let mut worn = held_item(
                protected.id,
                protected.name.as_deref().unwrap(),
                1,
                ItemContainer::Equipment,
            );
            worn.slot = slot;
            snapshot.seed_equipment(vec![worn]);
            let result = with_step(&snapshot, &mut ledger, 2, &banks, |cx| run.poll(cx));
            if protect_worn {
                assert!(matches!(result, Poll::Ready(Ok(_))));
                assert!(ledger
                    .as_ref()
                    .is_none_or(|ledger| ledger.outbox.is_empty()));
            } else {
                assert!(result.is_pending());
                let _ = with_step(&snapshot, &mut ledger, 3, &banks, |cx| run.poll(cx));
                assert!(matches!(
                    ledger.as_ref().unwrap().outbox[0].effect,
                    HostEffect::Interaction(crate::interact::InteractReq::Wear { .. })
                ));
            }
        }
    }
}

#[test]
fn loadout_acquisition_owner_stays_within_its_bounded_inline_bank_budget() {
    let run = std::mem::size_of::<LoadoutRun>();
    let wait = std::mem::size_of::<LoadoutObservationWait>();
    let survival = std::mem::size_of::<SurvivalLoadout>();
    eprintln!("LoadoutRun={run} LoadoutObservationWait={wait} SurvivalLoadout={survival}");
    // The inline 28-row capacity snapshot removes a per-check allocation:
    // +208 B, with LoadoutRun at 1136 B on macOS and Linux.
    assert!(run <= 1152, "LoadoutRun is {run} bytes");
    assert!(survival <= 128, "SurvivalLoadout is {survival} bytes");
}
