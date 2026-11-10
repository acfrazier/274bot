use super::*;

fn held(id: i32, name: &str, slot: i32) -> ItemView {
    slotted(item(id, name, 1, ItemContainer::Inventory), slot)
}

fn side(pack: &[ItemView]) -> Vec<ItemView> {
    pack.iter()
        .map(|row| {
            deposit_side_row(
                row.def.id,
                row.def.name.as_deref().unwrap(),
                row.count,
                row.slot,
            )
        })
        .collect()
}

fn fixture() -> (GameSnapshot, Arc<[i32]>) {
    let mut keep: Vec<_> = (10_000..10_128).collect();
    keep.extend([303, 321]);
    let keep = Arc::from(keep);
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    let pack = vec![
        held(303, "Small fishing net", 0),
        held(379, "Lobster", 1),
        held(317, "Raw shrimps", 2),
        held(2309, "Bread", 3),
    ];
    snapshot.seed_bank_observation(10, 1, Some(Vec::new()), side(&pack));
    snapshot.seed_inventory(pack, 28);
    (snapshot, keep)
}

fn request(keep: &Arc<[i32]>, slots: u8) -> BankRequest {
    BankRequest {
        bank: None,
        action: BankAction::DepositCapacity {
            keep: Arc::clone(keep),
            extra_keep: Some(379),
            slots,
        },
        partial_ok: false,
        access: None,
    }
}

#[test]
fn large_capacity_keep_set_allocates_nothing_per_begin_or_scan() {
    eprintln!("ProtectedCounts={}", std::mem::size_of::<ProtectedCounts>());
    let (snapshot, keep) = fixture();
    assert!(keep.len() > MAX_MEMO);
    with_tick(&snapshot, &mut None, 1, |tick| {
        // A positive control prevents an inert counter from passing the proof.
        let positive = allocation_counter::measure(|| {
            let allocation = std::hint::black_box(Vec::<u8>::with_capacity(8));
            drop(allocation);
        });
        assert!(positive.count_total > 0);

        let view = tick.cx.snapshot();
        let pack = view.inventory().unwrap().value;
        let side = view.bank_side().unwrap().value;
        let allocations = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                let machine = BankMachine::begin(request(&keep, 1), &mut tick.cx).unwrap();
                let BankAction::DepositCapacity { keep: shared, .. } = &machine.request.action
                else {
                    unreachable!();
                };
                assert!(Arc::ptr_eq(&keep, shared));
                assert_eq!(machine.protected_unchanged(Some(pack)), Some(true));
                assert!(matches!(
                    ops::deposit_capacity_next(&keep, Some(379), Some(side), Some(pack), true),
                    DepositScan::Click(click) if click.id == 317
                ));
                std::hint::black_box(machine);
            }
        });
        assert_eq!(allocations, allocation_counter::AllocationInfo::default());
    });
}

#[test]
fn capacity_deposit_preserves_large_shared_keep_set_and_extra_food() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, keep) = fixture();
    let mut ledger = None;
    let (handle, started) = begin_bank(&snapshot, &mut ledger, request(&keep, 1));
    assert!(started.is_pending());
    assert!(handle
        .inspect(|machine| {
            matches!(&machine.request.action,
                BankAction::DepositCapacity { keep: shared, .. } if Arc::ptr_eq(&keep, shared))
        })
        .unwrap());
    assert!(matches!(
        acknowledge(&mut ledger, 1),
        HostEffect::Interaction(InteractReq::InvButton { id: 317, .. })
    ));
    let pack = vec![
        held(303, "Small fishing net", 0),
        held(379, "Lobster", 1),
        held(2309, "Bread", 3),
    ];
    snapshot.seed_bank_observation(
        10,
        2,
        Some(vec![item(317, "Raw shrimps", 1, ItemContainer::Bank)]),
        side(&pack),
    );
    snapshot.seed_inventory(pack, 28);
    let settled = with_tick(&snapshot, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    });
    assert!(
        matches!(
            settled,
            Poll::Ready(Ok(BankReceipt {
                complete: true,
                deposited: 1
            }))
        ),
        "{settled:?}"
    );
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
}

#[test]
fn full_pack_capacity_deposit_keeps_every_held_food_during_gear_withdrawal() {
    let keep: Arc<[i32]> = Arc::from([379, 2309, 317]);
    let mut pack = vec![
        held(379, "Lobster", 0),
        held(2309, "Bread", 1),
        held(317, "Raw shrimps", 2),
    ];
    pack.extend((3..28).map(|slot| held(4_000 + slot, "Unprotected item", slot)));
    assert_eq!(pack.len(), 28);
    let side = side(&pack);

    assert!(matches!(
        ops::deposit_capacity_next(&keep, None, Some(&side), Some(&pack), true),
        DepositScan::Click(click) if click.id == 4_003
    ));
    assert!(keep
        .iter()
        .all(|food| pack.iter().any(|row| row.def.id == *food)));
}
#[test]
fn capacity_deposit_stops_when_previously_absent_kept_id_arrives() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, keep) = fixture();
    let mut ledger = None;
    let (handle, started) = begin_bank(&snapshot, &mut ledger, request(&keep, 2));
    assert!(started.is_pending());
    assert!(matches!(
        acknowledge(&mut ledger, 1),
        HostEffect::Interaction(InteractReq::InvButton { id: 317, .. })
    ));
    let pack = vec![
        held(303, "Small fishing net", 0),
        held(379, "Lobster", 1),
        held(2309, "Bread", 3),
        held(321, "Raw anchovies", 4),
    ];
    snapshot.seed_bank_observation(
        10,
        2,
        Some(vec![item(317, "Raw shrimps", 1, ItemContainer::Bank)]),
        side(&pack),
    );
    snapshot.seed_inventory(pack, 28);
    let settled = with_tick(&snapshot, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    });
    assert!(
        matches!(&settled,
            Poll::Ready(Err(ActionError::Blocked(reason)))
                if reason.as_ref() == "protected inventory changed during bank deposit"
        ),
        "{settled:?}"
    );
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    assert_eq!(ops::count_id(snapshot.inventory(), 321), 1);
    assert_eq!(ops::count_id(snapshot.inventory(), 379), 1);
    assert_eq!(ops::count_id(snapshot.inventory(), 2309), 1);
}
