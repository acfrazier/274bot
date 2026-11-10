//! `compat_v1_bank__sortbank__contract`: the native `sortBank` seat over the
//! shared planner (census rows `debt:008`, `catalog-export:049`).

use super::*;
use crate::native::{HostEffect, InteractionReceipt};
use crate::quester::families::tests::with_tick;
use api::obj_names::ItemDefView;
use api::quest_progress::EvidenceStamp;
use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer, VarpView};

fn banked(id: i32, name: &str, slot: i32) -> ItemView {
    ItemView {
        def: ItemDefView {
            id,
            name: Some(name.to_owned()),
            stackable: false,
            members: false,
            base_value: 0,
            noted: false,
            certificate_link: -1,
            certificate_template: -1,
        },
        container: ItemContainer::Bank,
        action_family: ItemActionFamily::Component,
        slot,
        count: 1,
        actions: Vec::new(),
        component_id: 5382,
    }
}

fn bank(rows: Vec<ItemView>, insert: i32) -> GameSnapshot {
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_inventory(Vec::new(), 28);
    snapshot.seed_bank_observation(5382, 41, Some(rows), Vec::new());
    snapshot.seed_varps(vec![VarpView {
        index: BANK_INSERT_VARP,
        value: insert,
    }]);
    snapshot
}

fn args(log: bool) -> (SortArgs, Arc<Mutex<TransferHooks>>) {
    let hooks = Arc::new(Mutex::new(TransferHooks::default()));
    (
        SortArgs {
            overrides: Vec::new(),
            hooks: Arc::clone(&hooks),
            log,
        },
        hooks,
    )
}

type Ledger = Option<Box<crate::native::ledger::Ledger>>;

fn begin(
    snapshot: &GameSnapshot,
    ledger: &mut Ledger,
    args: SortArgs,
) -> crate::native::ActionHandle<Sort> {
    with_tick(snapshot, ledger, 1, |tick| {
        tick.actions.begin::<Sort>(args, &mut tick.cx).unwrap()
    })
}

fn poll(
    snapshot: &GameSnapshot,
    ledger: &mut Ledger,
    handle: &crate::native::ActionHandle<Sort>,
    tick: u64,
) -> Poll<Result<Value, ActionError>> {
    with_tick(snapshot, ledger, tick, |native| {
        native.actions.poll(handle, &mut native.cx)
    })
}

/// Take the one queued interaction and record its host receipt.
fn acknowledge(ledger: &mut Ledger, tick: u64, accepted: bool) -> InteractReq {
    let ledger = ledger.as_mut().expect("an action ledger");
    let action = ledger.outbox.remove(0);
    ledger.complete_interaction(
        &action.authority(),
        InteractionReceipt {
            request_id: action.request_id.get(),
            evidence: EvidenceStamp {
                run: action.run(),
                tick,
                sequence: tick,
            },
            accepted,
            chat_since: 0,
        },
    );
    match action.effect {
        HostEffect::Interaction(request) => request,
        _ => panic!("expected an interaction"),
    }
}

fn result(poll: Poll<Result<Value, ActionError>>) -> Value {
    match poll {
        Poll::Ready(Ok(value)) => value,
        other => panic!("expected a sort result, got pending={}", other.is_pending()),
    }
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_bank__sortbank__contract() {
    if !api::test_support::game_data_available() {
        return;
    }
    // Idle results: the bank must be open and its item table loaded.
    let mut ledger = None;
    let mut closed = GameSnapshot::new();
    closed.seed_ingame(2);
    let handle = begin(&closed, &mut ledger, args(false).0);
    assert_eq!(
        result(poll(&closed, &mut ledger, &handle, 1)),
        json!({"sorted": false, "moves": 0, "mode": null, "unmatched": [], "reason": "bank not open"})
    );
    let mut ledger = None;
    let mut unloaded = GameSnapshot::new();
    unloaded.seed_ingame(2);
    unloaded.seed_bank_observation(5382, 41, None, Vec::new());
    let handle = begin(&unloaded, &mut ledger, args(false).0);
    assert_eq!(
        result(poll(&unloaded, &mut ledger, &handle, 1))["reason"],
        "snapshot not ready"
    );

    // An already sorted bank sends nothing.
    let mut ledger = None;
    let sorted = bank(
        vec![banked(436, "Copper ore", 0), banked(438, "Tin ore", 1)],
        0,
    );
    let handle = begin(&sorted, &mut ledger, args(false).0);
    assert_eq!(
        result(poll(&sorted, &mut ledger, &handle, 1)),
        json!({"sorted": true, "moves": 0, "mode": null, "unmatched": [], "reason": "sorted"})
    );
    assert!(ledger
        .as_ref()
        .is_none_or(|ledger| ledger.outbox.is_empty()));

    // Two ores out of order: one swap drag of the exact posted row, then the
    // next observed tick verifies the new order.
    let mut ledger = None;
    let unsorted = bank(
        vec![banked(438, "Tin ore", 0), banked(436, "Copper ore", 1)],
        0,
    );
    let (sort_args, hooks) = args(true);
    let handle = begin(&unsorted, &mut ledger, sort_args);
    assert!(poll(&unsorted, &mut ledger, &handle, 2).is_pending());
    let drag = acknowledge(&mut ledger, 2, true);
    assert!(
        matches!(
            drag,
            InteractReq::BankDrag {
                insert: false,
                component: 5382,
                ..
            }
        ),
        "a swap drag of a posted bank row: {drag:?}"
    );
    let InteractReq::BankDrag {
        id,
        slot,
        target_slot,
        ..
    } = drag
    else {
        unreachable!()
    };
    assert_eq!((id, slot, target_slot), (436, 1, 0));
    // The drag has no effect until a later observed tick.
    assert!(poll(&unsorted, &mut ledger, &handle, 2).is_pending());
    let after = bank(
        vec![banked(436, "Copper ore", 0), banked(438, "Tin ore", 1)],
        0,
    );
    assert!(poll(&after, &mut ledger, &handle, 3).is_pending());
    let log = {
        let mut hooks = hooks.lock().unwrap();
        let call = hooks.pending.take().expect("round log");
        hooks.answer = Some(TransferAnswer::Value(Value::Null));
        call.args[0].clone()
    };
    assert_eq!(log, json!("bank sort: round 1, swap, 1 sent, 1 to go"));
    assert_eq!(
        result(poll(&after, &mut ledger, &handle, 3)),
        json!({"sorted": true, "moves": 1, "mode": "swap", "unmatched": [], "reason": "sorted"})
    );

    // A refused drag that leaves the order unchanged twice reports no
    // progress and restores nothing (the mode never changed).
    let mut ledger = None;
    let handle = begin(&unsorted, &mut ledger, args(false).0);
    for tick in [2, 4] {
        assert!(poll(&unsorted, &mut ledger, &handle, tick).is_pending());
        let _ = acknowledge(&mut ledger, tick, true);
        assert!(poll(&unsorted, &mut ledger, &handle, tick).is_pending());
    }
    let stalled = result(poll(&unsorted, &mut ledger, &handle, 5));
    assert_eq!(stalled["reason"], "no progress");
    assert_eq!(stalled["sorted"], false);
    assert_eq!(stalled["moves"], 2);
}

#[test]
fn sort_presses_the_arrange_button_and_falls_back_to_swaps_when_it_never_sticks() {
    if !api::test_support::game_data_available() {
        return;
    }
    // A rotation: one insert beats four swaps, so the plan asks for insert
    // mode while the varp says swap.
    let names = ["Copper ore", "Tin ore", "Iron ore", "Silver ore", "Coal"];
    let ids = [436, 438, 440, 442, 453];
    let rotated = |first: usize| {
        (0..5)
            .map(|slot| {
                let index = (slot + first) % 5;
                banked(ids[index], names[index], slot as i32)
            })
            .collect::<Vec<_>>()
    };
    let unsorted = bank(rotated(1), 0);
    let mut ledger = None;
    let (sort_args, hooks) = args(true);
    let handle = begin(&unsorted, &mut ledger, sort_args);
    assert!(poll(&unsorted, &mut ledger, &handle, 2).is_pending());
    assert!(matches!(
        acknowledge(&mut ledger, 2, true),
        InteractReq::IfButton {
            component_id: ARRANGE_INSERT_COM
        }
    ));
    // The varp never changes: after the observed-tick budget the sort logs
    // and plans swaps instead.
    let deadline = 2 + crate::bank::ops::TRANSFER_OBSERVED_TICK_LIMIT;
    assert!(poll(&unsorted, &mut ledger, &handle, deadline - 1).is_pending());
    assert!(poll(&unsorted, &mut ledger, &handle, deadline).is_pending());
    let message = {
        let mut hooks = hooks.lock().unwrap();
        let call = hooks.pending.take().expect("fallback log");
        hooks.answer = Some(TransferAnswer::Value(Value::Null));
        call.args[0].clone()
    };
    assert_eq!(
        message,
        json!("bank sort: arrange mode did not stick, falling back to swaps")
    );
    assert!(poll(&unsorted, &mut ledger, &handle, deadline + 1).is_pending());
    assert!(matches!(
        acknowledge(&mut ledger, deadline + 1, true),
        InteractReq::BankDrag { insert: false, .. }
    ));
}

#[test]
fn sort_restores_the_original_arrange_mode() {
    if !api::test_support::game_data_available() {
        return;
    }
    // Insert mode set, but the plan uses swaps (two rows): the sort selects
    // swap, then puts insert back.
    let unsorted = bank(
        vec![banked(438, "Tin ore", 0), banked(436, "Copper ore", 1)],
        1,
    );
    let mut ledger = None;
    let handle = begin(&unsorted, &mut ledger, args(false).0);
    assert!(poll(&unsorted, &mut ledger, &handle, 2).is_pending());
    assert!(matches!(
        acknowledge(&mut ledger, 2, true),
        InteractReq::IfButton {
            component_id: ARRANGE_SWAP_COM
        }
    ));
    let swapped = bank(
        vec![banked(438, "Tin ore", 0), banked(436, "Copper ore", 1)],
        0,
    );
    assert!(poll(&swapped, &mut ledger, &handle, 3).is_pending());
    let _ = acknowledge(&mut ledger, 3, true);
    let done = bank(
        vec![banked(436, "Copper ore", 0), banked(438, "Tin ore", 1)],
        0,
    );
    assert!(poll(&done, &mut ledger, &handle, 4).is_pending());
    assert!(matches!(
        acknowledge(&mut ledger, 4, true),
        InteractReq::IfButton {
            component_id: ARRANGE_INSERT_COM
        }
    ));
    let restored = bank(
        vec![banked(436, "Copper ore", 0), banked(438, "Tin ore", 1)],
        1,
    );
    let value = result(poll(&restored, &mut ledger, &handle, 5));
    assert_eq!(value["sorted"], true);
    assert_eq!(value["mode"], "swap");
}

/// A valid 288-row bank can leave more than the channel's 128-entry array
/// limit unmatched; the result still crosses the seat channel intact.
#[test]
fn sort_results_with_more_unmatched_ids_than_one_channel_array_transport() {
    for count in [128_usize, 129, 288] {
        let ids: Vec<i32> = (0..count as i32).map(|id| 30_000 + id).collect();
        let value = sort_result(false, 7, Some(BankSortMode::Insert), &ids, "no progress");
        let bytes = crate::channel::encode(&value)
            .unwrap_or_else(|error| panic!("{count} unmatched ids encode: {error}"));
        let decoded = crate::channel::decode(&bytes).expect("the result decodes");
        let joined: Vec<f64> = decoded["unmatched"]
            .as_array()
            .expect("chunks")
            .iter()
            .flat_map(|chunk| chunk.as_array().expect("one chunk").iter())
            .map(|id| id.as_f64().expect("an id"))
            .collect();
        assert_eq!(
            joined,
            ids.iter().map(|&id| f64::from(id)).collect::<Vec<_>>()
        );
        assert_eq!(decoded["moves"].as_f64(), Some(7.0));
        assert_eq!(decoded["mode"], "insert");
        assert_eq!(decoded["reason"], "no progress");
    }
}
