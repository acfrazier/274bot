use super::{BankFetchCursor, BankFetchPlan};
use crate::bank_fetch::{BankStep, PlanningRowsCursor};
use crate::collision::{pack_walk, WorldCollision};
use crate::pack::{BankAccess, BankStand};
use crate::router::MissingReq;
use crate::world_state::WorldState;
use api::snapshot::WorldTile;

fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

fn collision() -> WorldCollision {
    let (walk, blocked) = pack_walk(&vec![0; 4 * 8 * 8]);
    WorldCollision {
        origin: tile(0, 0),
        width: 8,
        height: 8,
        walk,
        blocked,
        flags: None,
    }
}

fn stands() -> [BankStand; 1] {
    [BankStand {
        name: "Bank booth".into(),
        tile: tile(4, 4),
        access: BankAccess::Booth { op: 2 },
    }]
}

fn finish(
    cursor: &mut BankFetchCursor,
    missing: &[MissingReq],
    state: &WorldState,
    bank: &[(i32, i32)],
) -> Option<BankFetchPlan> {
    let collision = collision();
    let stands = stands();
    let heap_bound = BankFetchCursor::heap_bound(bank.len(), missing.len(), stands.len()).unwrap();
    let steps_bound = BankFetchPlan::steps_heap_bound(missing.len()).unwrap();
    let plan_bound = BankFetchPlan::heap_bound(missing.len()).unwrap();
    assert!(plan_bound <= heap_bound);
    for _ in 0..10_000 {
        let done = cursor.advance(missing, state, bank, &stands, tile(0, 0), &collision);
        assert!(cursor.retained_bytes() <= cursor.peak_retained_bytes());
        assert!(cursor.peak_retained_bytes() <= heap_bound);
        if done {
            let plan = cursor.take_plan();
            if let Some(plan) = plan.as_ref() {
                assert!(plan.retained_bytes() > 0);
                assert!(plan.retained_bytes() <= plan_bound);
                assert!(super::vec_bytes(&plan.steps) <= steps_bound);
                assert_eq!(cursor.retained_bytes(), 0, "selection scratch is released");
            }
            return plan;
        }
    }
    panic!("the finite fixture must settle within its unit bound");
}

#[test]
fn bank_rows_are_resumable_and_the_first_duplicate_remains_authoritative() {
    let state = WorldState::default();
    let missing = [MissingReq::WearAny { ids: vec![10] }];
    let mut bank: Vec<_> = (100..1100).map(|id| (id, 1)).collect();
    bank.extend([(10, -1), (10, 5)]);
    let collision = collision();
    let stands = stands();
    let mut cursor = BankFetchCursor::new();
    for _ in 0..bank.len() {
        assert!(!cursor.advance(&missing, &state, &bank, &stands, tile(0, 0), &collision));
        assert!(cursor.take_plan().is_none());
    }
    assert!(finish(&mut cursor, &missing, &state, &bank).is_none());
    assert!(cursor.advance(&missing, &state, &bank, &stands, tile(0, 0), &collision));
}

#[test]
fn selection_and_each_owned_state_delta_keep_the_legacy_order() {
    let mut state = WorldState::default();
    state.inv.extend([(10, 2), (20, 3), (99, 1)]);
    state.worn.insert(50);
    state.quests.insert("retained quest evidence".into());
    let quest_storage = state.quests.iter().next().unwrap().as_ptr();
    let missing = [
        MissingReq::Carry { id: 10, count: 2 },
        MissingReq::Carry { id: 10, count: 3 },
        MissingReq::WearAny { ids: vec![30, 20] },
        MissingReq::WearAny { ids: vec![20, 40] },
        MissingReq::WearAny { ids: vec![10] },
        MissingReq::WearAny { ids: vec![50] },
        MissingReq::Carry { id: 11, count: 0 },
        MissingReq::Carry { id: 12, count: 7 },
    ];
    let bank = [(10, 5), (10, 100), (30, 1), (12, 20)];
    let mut cursor = BankFetchCursor::new();
    let mut plan = finish(&mut cursor, &missing, &state, &bank).unwrap();
    assert!(cursor.take_plan().is_none(), "plan transfer is exact-once");
    let steps = vec![
        BankStep::Walk {
            x: 3,
            z: 4,
            level: 0,
        },
        BankStep::Open,
        BankStep::WithdrawX { id: 10 },
        BankStep::WithdrawXAmount { id: 10, count: 2 },
        BankStep::WithdrawX { id: 12 },
        BankStep::WithdrawXAmount { id: 12, count: 7 },
        BankStep::Wear { id: 20 },
        BankStep::Wear { id: 10 },
    ];
    assert_eq!(plan.steps(), steps);
    assert_eq!(
        state.inv.get(&10),
        Some(&2),
        "selection does not apply deltas"
    );
    assert!(!plan.apply_next(&mut state));
    assert_eq!(state.inv.get(&10), Some(&4));
    assert!(!state.inv.contains_key(&12));
    assert!(!plan.apply_next(&mut state));
    assert_eq!(state.inv.get(&12), Some(&7));
    assert!(!state.worn.contains(&20));
    assert!(!plan.apply_next(&mut state));
    assert_eq!(state.inv.get(&20), Some(&2));
    assert!(state.worn.contains(&20));
    assert!(!state.worn.contains(&10));
    assert!(plan.apply_next(&mut state));
    assert_eq!(state.inv.get(&10), Some(&3));
    assert!(state.worn.contains(&10));
    assert_eq!(state.inv.get(&99), Some(&1));
    assert_eq!(state.quests.iter().next().unwrap().as_ptr(), quest_storage);
    let settled = state.clone();
    assert!(plan.apply_next(&mut state));
    assert_eq!(state, settled, "settled delta application is idempotent");
    assert_eq!(plan.into_steps(), steps);
}

#[test]
fn empty_or_unavailable_worn_alternatives_settle_without_a_partial_plan() {
    let state = WorldState::default();
    for missing in [
        vec![],
        vec![MissingReq::WearAny { ids: vec![] }],
        vec![MissingReq::Carry { id: 10, count: 2 }],
        vec![MissingReq::WearAny { ids: vec![10] }],
    ] {
        assert!(finish(&mut BankFetchCursor::new(), &missing, &state, &[]).is_none());
    }
}

#[test]
fn hinted_overlay_resumes_copy_and_shifts_and_checks_only_original_rows() {
    let rows = [(10, -5), (40, 2), (100, 1)];
    let missing = [
        MissingReq::Carry { id: 5, count: 3 },
        MissingReq::Carry { id: 10, count: 2 },
        MissingReq::Carry { id: 5, count: 4 },
        MissingReq::WearAny { ids: vec![10] },
        MissingReq::WearAny { ids: vec![10] },
        MissingReq::WearAny { ids: vec![40, 7] },
        MissingReq::WearAny { ids: vec![9, 10] },
        MissingReq::WearAny { ids: vec![] },
        MissingReq::Carry { id: -1, count: -1 },
    ];
    let mut cursor = PlanningRowsCursor::new();
    for _ in 0..=rows.len() {
        assert!(!cursor.advance(&rows, &missing));
        assert!(cursor.take_rows().is_none());
    }
    let mut units = 0;
    while !cursor.advance(&rows, &missing) {
        units += 1;
        assert!(units < 1_000);
    }
    assert!(units > 30, "matching and shifts must span bounded units");
    let retained = cursor.retained_bytes();
    assert!(
        cursor.peak_retained_bytes() > retained,
        "growth includes the old buffer"
    );
    let planned = cursor.take_rows().unwrap();
    assert_eq!(planned, [(5, 4), (9, 1), (10, 4), (40, 2), (100, 1)]);
    assert_eq!(super::vec_bytes(&planned), retained);
    assert_eq!(cursor.retained_bytes(), 0);
    assert!(cursor.take_rows().is_none());
    assert!(cursor.advance(&rows, &missing));
    assert_eq!(rows, [(10, -5), (40, 2), (100, 1)]);
}

#[test]
fn planner_peak_includes_the_old_vector_at_each_capacity_growth() {
    let state = WorldState::default();
    let missing = [MissingReq::Carry { id: 0, count: 1 }];
    let bank: Vec<_> = (0..17).map(|id| (id, 1)).collect();
    let collision = collision();
    let stands = stands();
    let mut cursor = BankFetchCursor::new();
    let mut growths = 0;
    for _ in 0..100 {
        let old = super::vec_bytes(&cursor.bank_counts);
        let done = cursor.advance(&missing, &state, &bank, &stands, tile(0, 0), &collision);
        if super::vec_bytes(&cursor.bank_counts) > old {
            growths += 1;
            assert!(
                cursor.peak_retained_bytes() >= cursor.retained_bytes() + old,
                "the new allocation must not hide its still-live predecessor"
            );
        }
        if done {
            break;
        }
    }
    assert!(growths >= 3);
}

#[test]
fn admission_bounds_cover_growth_and_transferred_outputs() {
    let state = WorldState::default();
    for count in [0, 1, 2, 3, 5, 8, 9, 17, 33] {
        let bank: Vec<_> = (0..2 * count).rev().map(|id| (id, 20)).collect();
        let missing: Vec<_> = (0..count)
            .flat_map(|id| {
                [
                    MissingReq::Carry { id, count: 3 },
                    MissingReq::WearAny {
                        ids: vec![count + id],
                    },
                ]
            })
            .collect();
        let mut cursor = BankFetchCursor::new();
        let plan = finish(&mut cursor, &missing, &state, &bank);
        assert_eq!(plan.is_some(), count > 0);

        // An odd exact starting capacity must also fit after later growth.
        let rows = [(1000, 1), (1001, 1), (1002, 1), (1003, 1), (1004, 1)];
        let bound = PlanningRowsCursor::heap_bound(rows.len(), missing.len()).unwrap();
        let rows_bound = PlanningRowsCursor::rows_heap_bound(rows.len(), missing.len()).unwrap();
        assert_eq!(rows_bound.checked_mul(2), Some(bound));
        let mut overlay = PlanningRowsCursor::new();
        let mut units = 0;
        while !overlay.advance(&rows, &missing) {
            units += 1;
            assert!(units < 10_000);
            assert!(overlay.peak_retained_bytes() <= bound);
        }
        assert!(overlay.peak_retained_bytes() <= bound);
        let planned = overlay.take_rows().unwrap();
        assert!(super::vec_bytes(&planned) <= rows_bound);
        assert_eq!(overlay.retained_bytes(), 0);
    }
}

#[test]
fn bounded_cursors_keep_repeated_and_overlapping_missing_rows_within_u_eff() {
    // Synthetic lease counts: C=3 carry rows, W=10 worn IDs, M=C+5 settles.
    let carry_rows: usize = 3;
    let worn_rows = 10;
    let settled_rows = 5;
    let max_missing_requests = carry_rows.checked_add(settled_rows).unwrap();
    let item_rows = carry_rows
        .checked_add(worn_rows)
        .unwrap()
        .min(max_missing_requests);
    let state = WorldState::default();
    let bank = [(10, 20), (20, 20), (30, 20), (40, 20)];
    let missing = [
        MissingReq::Carry { id: 10, count: 1 },
        MissingReq::Carry { id: 10, count: 3 },
        MissingReq::Carry { id: 40, count: 2 },
        MissingReq::WearAny { ids: vec![10, 20] },
        MissingReq::WearAny { ids: vec![20, 30] },
        MissingReq::WearAny { ids: vec![30, 40] },
        MissingReq::WearAny { ids: vec![10, 20] },
        MissingReq::WearAny { ids: vec![30, 10] },
    ];
    assert_eq!(missing.len(), max_missing_requests);

    let mut cursor = BankFetchCursor::with_limits(bank.len(), item_rows, stands().len()).unwrap();
    assert_eq!(cursor.retained_bytes(), 0, "construction does not allocate");
    let plan = finish(&mut cursor, &missing, &state, &bank).unwrap();
    assert_eq!(
        plan.withdrawals.as_slice(),
        &[(10, 4), (40, 2), (20, 1), (30, 1)]
    );
    assert_eq!(plan.wear.as_slice(), &[10, 20, 30]);
    let mut unique_plan_ids: Vec<_> = plan
        .withdrawals
        .iter()
        .map(|&(id, _)| id)
        .chain(plan.wear.iter().copied())
        .collect();
    unique_plan_ids.sort_unstable();
    unique_plan_ids.dedup();
    assert_eq!(unique_plan_ids.as_slice(), &[10, 20, 30, 40]);
    assert!(unique_plan_ids.len() <= item_rows);
    assert!(
        cursor.peak_retained_bytes()
            <= BankFetchCursor::heap_bound(bank.len(), item_rows, stands().len()).unwrap()
    );
    assert!(plan.retained_bytes() <= BankFetchPlan::heap_bound(item_rows).unwrap());

    let mut planner = PlanningRowsCursor::with_limits(0, item_rows).unwrap();
    assert_eq!(
        planner.retained_bytes(),
        0,
        "construction does not allocate"
    );
    let planner_bound = PlanningRowsCursor::heap_bound(0, item_rows).unwrap();
    let mut units = 0;
    while !planner.advance(&[], &missing) {
        units += 1;
        assert!(units < 10_000);
        assert!(planner.peak_retained_bytes() <= planner_bound);
    }
    let planned = planner.take_rows().unwrap();
    assert_eq!(planned, [(10, 5), (20, 1), (30, 2), (40, 2)]);
    assert!(planned.len() <= item_rows);
    assert!(planner.peak_retained_bytes() <= planner_bound);
}

#[test]
fn bounded_bank_and_planned_row_buffers_use_bank_rows_independently() {
    let state = WorldState::default();
    let mut bank = Vec::with_capacity(65);
    bank.push((10, 1));
    bank.extend((100..164).map(|id| (id, 1)));
    let missing = [MissingReq::WearAny { ids: vec![10] }];
    assert!(bank.len() > 1);
    let mut cursor = BankFetchCursor::with_limits(bank.len(), 1, stands().len()).unwrap();
    let plan = finish(&mut cursor, &missing, &state, &bank).unwrap();
    assert_eq!(plan.wear.as_slice(), &[10]);

    assert!(
        finish(
            &mut BankFetchCursor::with_limits(bank.len() - 1, 1, stands().len()).unwrap(),
            &missing,
            &state,
            &bank,
        )
        .is_none(),
        "the bank index cannot exceed its captured bank-row count"
    );
    assert!(
        finish(
            &mut BankFetchCursor::with_limits(bank.len(), 0, stands().len()).unwrap(),
            &missing,
            &state,
            &bank,
        )
        .is_none(),
        "bank rows do not increase the unique plan-item limit"
    );

    let rows = bank.as_slice();
    let no_missing: [MissingReq; 0] = [];
    let mut planner = PlanningRowsCursor::with_limits(rows.len(), 0).unwrap();
    let bound = PlanningRowsCursor::heap_bound(rows.len(), 0).unwrap();
    let mut units = 0;
    while !planner.advance(rows, &no_missing) {
        units += 1;
        assert!(units < 10_000);
        assert!(planner.peak_retained_bytes() <= bound);
    }
    assert_eq!(planner.take_rows().unwrap(), rows);
    assert!(
        PlanningRowsCursor::with_limits(rows.len() - 1, 0)
            .unwrap()
            .advance(rows, &no_missing),
        "an over-limit original bank slice fails at preflight"
    );
}

#[test]
fn bounded_cursor_limits_fail_closed_on_overflow_and_item_growth() {
    assert!(BankFetchCursor::with_limits(usize::MAX, 1, 1).is_some());
    assert!(BankFetchCursor::with_limits(0, usize::MAX, 1).is_none());
    assert!(BankFetchCursor::with_limits(0, 1, usize::MAX).is_none());
    assert!(PlanningRowsCursor::with_limits(usize::MAX, 1).is_none());

    let state = WorldState::default();
    let missing = [MissingReq::Carry { id: 10, count: 1 }];
    let bank = [(10, 1)];
    assert!(finish(
        &mut BankFetchCursor::with_limits(bank.len(), 0, stands().len()).unwrap(),
        &missing,
        &state,
        &bank,
    )
    .is_none());

    let many_items = [
        MissingReq::Carry { id: 10, count: 1 },
        MissingReq::Carry { id: 20, count: 1 },
    ];
    let mut planner = PlanningRowsCursor::with_limits(0, 1).unwrap();
    let no_rows: [(i32, i32); 0] = [];
    let mut units = 0;
    while !planner.advance(&no_rows, &many_items) {
        units += 1;
        assert!(units < 100);
    }
    assert!(planner.take_rows().is_none());
}

#[test]
fn bounded_bulk_step_growth_fails_before_appending_a_partial_group() {
    let mut cursor = BankFetchCursor::with_limits(1, 1, 1).unwrap();
    cursor.phase = super::Phase::WithdrawSteps;
    cursor.bank_counts.push((10, 100));
    cursor.withdrawals.push((10, 20));
    cursor.step_rows_limit = Some(1);

    let collision = collision();
    let stands = stands();
    assert!(cursor.advance(
        &[],
        &WorldState::default(),
        &[],
        &stands,
        tile(0, 0),
        &collision,
    ));
    assert!(cursor.steps.is_empty());
    assert!(cursor.take_plan().is_none());
}

#[test]
fn admission_bounds_reject_unrepresentable_counts_without_clamping() {
    assert_eq!(BankFetchCursor::heap_bound(0, 0, 0), Some(0));
    assert_eq!(BankFetchPlan::heap_bound(0), Some(0));
    assert_eq!(PlanningRowsCursor::heap_bound(0, 0), Some(0));
    assert_eq!(PlanningRowsCursor::rows_heap_bound(0, 0), Some(0));
    assert_eq!(BankFetchCursor::heap_bound(usize::MAX, 1, 1), None);
    assert_eq!(BankFetchCursor::heap_bound(1, usize::MAX, 1), None);
    assert_eq!(BankFetchCursor::heap_bound(1, 1, usize::MAX), None);
    assert_eq!(BankFetchPlan::heap_bound(usize::MAX), None);
    assert_eq!(PlanningRowsCursor::heap_bound(usize::MAX, 1), None);
    assert_eq!(PlanningRowsCursor::heap_bound(1, usize::MAX), None);
    assert_eq!(PlanningRowsCursor::rows_heap_bound(usize::MAX, 1), None);
    assert_eq!(PlanningRowsCursor::rows_heap_bound(1, usize::MAX), None);
}

#[test]
fn access_goals_keep_air_order_and_tied_planes_then_transfer_targets() {
    let state = WorldState::default();
    let missing = [MissingReq::Carry { id: 10, count: 1 }];
    let bank = [(10, 1)];
    let collision = collision();
    let stands: Vec<_> = [1, 2]
        .into_iter()
        .map(|level| BankStand {
            name: "Bank booth".into(),
            tile: WorldTile { x: 4, z: 4, level },
            access: BankAccess::Booth { op: 2 },
        })
        .collect();
    let mut cursor = BankFetchCursor::with_limits(bank.len(), 1, stands.len()).unwrap();
    let bound = BankFetchCursor::heap_bound(bank.len(), 1, stands.len()).unwrap();
    for _ in 0..1_000 {
        if cursor.advance(&missing, &state, &bank, &stands, tile(0, 0), &collision) {
            break;
        }
        assert!(cursor.peak_retained_bytes() <= bound);
    }
    let expected: Vec<_> = [(3, 4), (4, 3), (4, 5), (5, 4)]
        .into_iter()
        .flat_map(|(x, z)| [1, 2].map(|level| WorldTile { x, z, level }))
        .collect();
    assert_eq!(cursor.access_targets(), expected);
    assert_eq!(cursor.planned_access(), expected.first().copied());
    assert!(cursor.accesses.is_empty(), "ranking scratch is released");
    let before = cursor.retained_bytes();
    let targets = cursor.take_access_targets();
    assert!(cursor.access_targets().is_empty());
    assert!(cursor.retained_bytes() < before);
    let chosen = targets[7];
    assert!(cursor.select_access(chosen));
    let plan = cursor.take_plan().unwrap();
    assert_eq!(
        plan.steps()[0],
        BankStep::Walk {
            x: chosen.x,
            z: chosen.z,
            level: chosen.level,
        }
    );
    assert_eq!(cursor.retained_bytes(), 0);
}

#[test]
fn access_goal_growth_respects_the_captured_stand_limit() {
    let mut cursor = BankFetchCursor::with_limits(1, 1, 0).unwrap();
    assert!(finish(
        &mut cursor,
        &[MissingReq::Carry { id: 10, count: 1 }],
        &WorldState::default(),
        &[(10, 1)],
    )
    .is_none());
}
