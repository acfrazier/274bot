use super::*;

const ITEM: i32 = 100;
const LOWER: i32 = 99;
const OTHER: i32 = 101;
const COINS: i32 = 995;

#[derive(Clone, Copy)]
struct CountsFixture<'a> {
    held: &'a [(i32, i32)],
    worn: &'a [(i32, i32)],
    banked: &'a [(i32, i32)],
    unknown_bank_ids: &'a [i32],
    origin: Origin,
    free_slots: Option<i32>,
}

impl Counts for CountsFixture<'_> {
    fn held(&self, id: i32) -> Option<i32> {
        Some(count(self.held, id))
    }

    fn worn(&self, id: i32) -> Option<i32> {
        Some(count(self.worn, id))
    }

    fn banked(&self, id: i32) -> Option<i32> {
        if self.origin == Origin::Unknown || self.unknown_bank_ids.contains(&id) {
            None
        } else {
            Some(count(self.banked, id))
        }
    }

    fn banked_origin(&self) -> Origin {
        self.origin
    }

    fn bank_seen(&self) -> Option<BankSeen> {
        (self.origin != Origin::Unknown).then_some(BankSeen {
            origin: self.origin,
            observed_at_unix: 1_700_000_000,
            rows: u16::try_from(self.banked.len()).unwrap_or(u16::MAX),
            count: 0,
        })
    }

    fn free_slots(&self) -> Option<i32> {
        self.free_slots
    }
}

fn count(rows: &[(i32, i32)], id: i32) -> i32 {
    rows.iter()
        .find_map(|(row_id, count)| (*row_id == id).then_some(*count))
        .unwrap_or(0)
}

fn fixture<'a>(
    held: &'a [(i32, i32)],
    banked: &'a [(i32, i32)],
    origin: Origin,
) -> CountsFixture<'a> {
    CountsFixture {
        held,
        worn: &[],
        banked,
        unknown_bank_ids: &[],
        origin,
        free_slots: Some(28),
    }
}

fn need(
    id: i32,
    target: i32,
    minimum: i32,
    presence: Presence,
    kind: NeedKind,
    sources: SourceSet,
) -> Need {
    Need {
        item: id,
        name: Arc::from(format!("item-{id}")),
        target,
        minimum,
        presence,
        kind,
        sources,
        alternatives: Arc::from([]),
    }
}

fn tier_need(
    id: i32,
    alternatives: &[i32],
    target: i32,
    presence: Presence,
    sources: SourceSet,
) -> Need {
    Need {
        alternatives: Arc::from(alternatives),
        ..need(id, target, target, presence, NeedKind::Required, sources)
    }
}

fn stamp() -> EvidenceStamp {
    EvidenceStamp {
        run: api::selected::RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        tick: 1,
        sequence: 1,
    }
}

fn request(needs: Vec<Need>) -> AcquireRequest {
    AcquireRequest {
        needs: needs.into(),
        deposit: None,
        bank: BankChoice {
            bank: None,
            explicit: None,
            required: false,
            preferences: BankPreferences::default(),
        },
        stage: BankStage::Planned,
        partial_ok: false,
        close: CloseBank::LeaveOpen,
        vendor: VendorChoice::None,
        coins: CoinPolicy::None,
        walk: WalkOptions::default(),
        required_after: stamp(),
    }
}

fn plan_for(request: &AcquireRequest, counts: &dyn Counts) -> AcquireVerdict {
    plan(request, counts, WorldFacts::default()).expect("bank-only planning is available")
}

fn planned(verdict: AcquireVerdict) -> AcquirePlan {
    match verdict {
        AcquireVerdict::Plan(plan) => plan,
        AcquireVerdict::Satisfied => panic!("expected a plan with work"),
        AcquireVerdict::Blocked(block) => panic!("expected a plan, got {:?}", block.reason),
    }
}

fn blocked(verdict: AcquireVerdict) -> AcquireBlock {
    match verdict {
        AcquireVerdict::Blocked(block) => block,
        AcquireVerdict::Satisfied => panic!("expected a block"),
        AcquireVerdict::Plan(_) => panic!("expected a block, got a work plan"),
    }
}

fn withdraw(id: i32, name: &str, qty: i32) -> BankAction {
    BankAction::Withdraw {
        item: crate::native_bank::BankItem {
            id,
            name: Arc::from(name),
        },
        qty,
    }
}

fn authored(actions: Vec<BankAction>, partial_ok: bool) -> AcquireRequest {
    let mut request = request(Vec::new());
    request.stage = BankStage::Authored(actions.into());
    request.partial_ok = partial_ok;
    request
}

#[test]
fn authored_stage_always_plans_including_empty_and_all_held_actions() {
    let counts = fixture(&[(ITEM, 10)], &[], Origin::Session);
    let empty = planned(plan_for(&authored(Vec::new(), false), &counts));
    assert!(empty.authored);
    assert_eq!(empty.withdrawal_count, 0);

    let held_action = planned(plan_for(
        &authored(vec![withdraw(ITEM, "item", 10)], false),
        &counts,
    ));
    assert!(
        held_action.authored,
        "authored actions still execute when the pack already holds the target"
    );
    assert_eq!(held_action.withdrawal_count, 0);
}

#[test]
fn authored_session_refusal_keeps_legacy_text_and_partial_rules() {
    let none = fixture(&[], &[], Origin::Session);
    let some = fixture(&[], &[(ITEM, 1)], Origin::Session);
    let multiple_shortages = authored(
        vec![withdraw(ITEM, "Item", 3), withdraw(OTHER, "Other", 4)],
        false,
    );
    assert_eq!(
        blocked(plan_for(&multiple_shortages, &none))
            .message
            .as_ref(),
        "need 3 Item; held 0, banked 0"
    );

    let partial_none = authored(
        vec![withdraw(ITEM, "Item", 3), withdraw(OTHER, "Other", 4)],
        true,
    );
    assert_eq!(
        blocked(plan_for(&partial_none, &none)).message.as_ref(),
        "need 3 Item; held 0, banked 0; need 4 Other; held 0, banked 0"
    );

    let partial_some = authored(
        vec![withdraw(ITEM, "Item", 3), withdraw(OTHER, "Other", 4)],
        true,
    );
    assert!(matches!(
        plan_for(&partial_some, &some),
        AcquireVerdict::Plan(_)
    ));

    let multiple_obtainable = fixture(&[], &[(ITEM, 1), (OTHER, 2)], Origin::Session);
    assert!(matches!(
        plan_for(
            &authored(
                vec![withdraw(ITEM, "Item", 3), withdraw(OTHER, "Other", 4)],
                true
            ),
            &multiple_obtainable
        ),
        AcquireVerdict::Plan(_)
    ));
}

#[test]
fn generated_empty_request_is_satisfied_but_deposit_only_is_work() {
    let counts = fixture(&[], &[], Origin::Session);
    assert!(matches!(
        plan_for(&request(Vec::new()), &counts),
        AcquireVerdict::Satisfied
    ));

    let mut deposit_request = request(Vec::new());
    deposit_request.deposit = Some(DepositSpec::Ids {
        products: Arc::from([ITEM]),
        keep: Arc::from([COINS]),
    });
    let deposit_plan = planned(plan_for(&deposit_request, &counts));
    assert_eq!(deposit_plan.withdrawal_count, 0);
    assert!(!deposit_plan.authored);
}

#[test]
fn scan_precedes_an_uncovered_need_and_block_precedes_deposit() {
    let unknown = fixture(&[], &[], Origin::Unknown);
    let mut needs = request(vec![
        need(
            ITEM,
            1,
            1,
            Presence::Held,
            NeedKind::Required,
            SourceSet::PACK | SourceSet::BANK,
        ),
        need(
            OTHER,
            1,
            1,
            Presence::Held,
            NeedKind::Required,
            SourceSet::PACK,
        ),
    ]);
    let scan = planned(plan_for(&needs, &unknown));
    assert!(scan.scan, "the unknown bank scan must be real work");

    needs.deposit = Some(DepositSpec::Capacity {
        keep: Arc::from([]),
        slots: 1,
    });
    let session = fixture(&[], &[], Origin::Session);
    let block = blocked(plan_for(&needs, &session));
    assert_eq!(block.need, 0, "the first required shortage wins");
    assert_eq!(block.reason, BlockReason::SupplyMissing);
}

#[test]
fn usable_partial_stock_is_withdrawn_to_the_final_held_count() {
    let need = need(
        ITEM,
        10,
        5,
        Presence::Held,
        NeedKind::Required,
        SourceSet::PACK | SourceSet::BANK,
    );
    let counts = fixture(&[(ITEM, 3)], &[(ITEM, 2)], Origin::Session);
    let result = planned(plan_for(&request(vec![need]), &counts));
    assert_eq!(result.withdrawal_count, 1);
    let row = result.withdrawals[0].as_ref().expect("one withdrawal");
    assert_eq!((row.id, row.target), (ITEM, 5));
}

#[test]
fn usable_minimum_allows_satisfied_and_short_target_is_reported_when_other_work_exists() {
    let counts = fixture(&[(ITEM, 5)], &[(ITEM, 0), (OTHER, 1)], Origin::Session);
    let short_need = need(
        ITEM,
        10,
        5,
        Presence::Held,
        NeedKind::Required,
        SourceSet::PACK | SourceSet::BANK,
    );
    assert!(matches!(
        plan_for(&request(vec![short_need.clone()]), &counts),
        AcquireVerdict::Satisfied
    ));

    let mut planner = Planner::new(&counts, WorldFacts::default());
    planner.require(&short_need).expect("short usable need");
    assert_eq!(
        planner.plan.looked.short_of_target, 1,
        "the standalone usable need records its short target"
    );
    let companion = need(
        OTHER,
        1,
        1,
        Presence::Held,
        NeedKind::Required,
        SourceSet::PACK | SourceSet::BANK,
    );
    planner.require(&companion).expect("companion need");
    let plan = planned(planner.verdict(None));
    assert_eq!(plan.looked.short_of_target, 1);
}

#[test]
fn required_shortage_blocks_at_the_need_index_while_optional_shortage_adds_no_work() {
    let counts = fixture(&[(ITEM, 1)], &[(ITEM, 1), (OTHER, 0)], Origin::Session);
    let first = need(
        ITEM,
        1,
        1,
        Presence::Held,
        NeedKind::Required,
        SourceSet::PACK,
    );
    let second = need(
        OTHER,
        2,
        2,
        Presence::Held,
        NeedKind::Required,
        SourceSet::PACK | SourceSet::BANK,
    );
    let block = blocked(plan_for(&request(vec![first, second]), &counts));
    assert_eq!(block.need, 1);
    assert_eq!(block.reason, BlockReason::SupplyMissing);

    let optional = need(
        OTHER,
        2,
        2,
        Presence::Held,
        NeedKind::Optional,
        SourceSet::PACK | SourceSet::BANK,
    );
    assert!(matches!(
        plan_for(&request(vec![optional]), &counts),
        AcquireVerdict::Satisfied
    ));
}

#[test]
fn optional_partial_bank_keeps_its_withdrawal_without_blocking() {
    let optional = need(
        ITEM,
        10,
        10,
        Presence::Held,
        NeedKind::Optional,
        SourceSet::PACK | SourceSet::BANK,
    );
    let counts = fixture(&[(ITEM, 3)], &[(ITEM, 2)], Origin::Session);
    let result = planned(plan_for(&request(vec![optional]), &counts));
    assert_eq!(result.withdrawal_count, 1);
    assert_eq!(
        result.withdrawals[0]
            .as_ref()
            .expect("partial withdrawal")
            .target,
        5
    );
}

#[test]
fn optional_shortage_with_unknown_bank_scans_but_optional_session_shortage_does_not() {
    let optional = need(
        ITEM,
        2,
        2,
        Presence::Held,
        NeedKind::Optional,
        SourceSet::PACK | SourceSet::BANK,
    );
    let unknown = fixture(&[], &[], Origin::Unknown);
    assert!(planned(plan_for(&request(vec![optional.clone()]), &unknown)).scan);

    let session = fixture(&[], &[(ITEM, 0)], Origin::Session);
    assert!(matches!(
        plan_for(&request(vec![optional]), &session),
        AcquireVerdict::Satisfied
    ));
}

#[test]
fn hint_shortage_verifies_even_when_current_count_meets_minimum() {
    let need = need(
        ITEM,
        10,
        5,
        Presence::Held,
        NeedKind::Required,
        SourceSet::PACK | SourceSet::BANK,
    );
    let hint = fixture(&[(ITEM, 3)], &[(ITEM, 1)], Origin::Hint);
    let result = planned(plan_for(&request(vec![need]), &hint));
    assert!(
        result.scan,
        "a target shortage under Hint must visit the bank despite meeting minimum"
    );
    assert_eq!(result.withdrawal_count, 1);
    assert_eq!(
        result.withdrawals[0]
            .as_ref()
            .expect("banked lower-up count")
            .target,
        4
    );
}

#[test]
fn presence_distinguishes_held_carried_and_wielded() {
    let counts = CountsFixture {
        held: &[(ITEM, 1)],
        worn: &[(ITEM, 1)],
        banked: &[],
        unknown_bank_ids: &[],
        origin: Origin::Session,
        free_slots: Some(28),
    };
    let held_only = need(
        ITEM,
        2,
        2,
        Presence::Held,
        NeedKind::Required,
        SourceSet::PACK | SourceSet::WORN,
    );
    assert_eq!(
        blocked(plan_for(&request(vec![held_only]), &counts)).reason,
        BlockReason::SupplyMissing
    );

    let carried = need(
        ITEM,
        2,
        2,
        Presence::Carried,
        NeedKind::Required,
        SourceSet::PACK | SourceSet::WORN,
    );
    assert!(matches!(
        plan_for(&request(vec![carried]), &counts),
        AcquireVerdict::Satisfied
    ));

    let wielded = need(
        ITEM,
        2,
        2,
        Presence::Wielded,
        NeedKind::Required,
        SourceSet::PACK | SourceSet::WORN,
    );
    assert!(matches!(
        plan_for(&request(vec![wielded]), &counts),
        AcquireVerdict::Satisfied
    ));
}

#[test]
fn held_unworn_wielded_tool_plans_only_one_wear_and_worn_tool_is_satisfied() {
    let held = fixture(&[(ITEM, 1)], &[], Origin::Session);
    let wielded = need(
        ITEM,
        1,
        1,
        Presence::Wielded,
        NeedKind::Required,
        SourceSet::PACK | SourceSet::WORN,
    );
    let wear_plan = planned(plan_for(&request(vec![wielded.clone()]), &held));
    assert_eq!(wear_plan.wear_count, 1);
    assert_eq!(wear_plan.wears[0], Some(ITEM));
    assert_eq!(wear_plan.withdrawal_count, 0);
    let lower_held = fixture(&[(LOWER, 1)], &[], Origin::Session);
    let lower_wielded = tier_need(
        ITEM,
        &[LOWER],
        1,
        Presence::Wielded,
        SourceSet::PACK | SourceSet::WORN,
    );
    let lower_plan = planned(plan_for(&request(vec![lower_wielded]), &lower_held));
    assert_eq!(
        lower_plan.wear_count, 1,
        "an acceptable lower-tier item also needs only wear work"
    );
    assert_eq!(lower_plan.wears[0], Some(LOWER));

    let worn = CountsFixture {
        held: &[],
        worn: &[(ITEM, 1)],
        banked: &[],
        unknown_bank_ids: &[],
        origin: Origin::Session,
        free_slots: Some(28),
    };
    assert!(matches!(
        plan_for(&request(vec![wielded]), &worn),
        AcquireVerdict::Satisfied
    ));
}

#[test]
fn planner_uses_a_banked_lower_acceptable_tier() {
    let need = tier_need(
        ITEM,
        &[LOWER],
        1,
        Presence::Wielded,
        SourceSet::PACK | SourceSet::WORN | SourceSet::BANK,
    );
    let counts = fixture(&[], &[(LOWER, 1)], Origin::Session);
    let result = planned(plan_for(&request(vec![need]), &counts));
    assert_eq!(result.withdrawal_count, 1);
    let row = result.withdrawals[0]
        .as_ref()
        .expect("lower tier withdrawal");
    assert_eq!((row.id, row.target), (LOWER, 1));
}

#[test]
fn source_permissions_are_per_need_and_pack_only_coin_policy_ignores_bank_coins() {
    let needs = vec![
        need(
            ITEM,
            1,
            1,
            Presence::Held,
            NeedKind::Required,
            SourceSet::PACK,
        ),
        need(
            OTHER,
            1,
            1,
            Presence::Held,
            NeedKind::Required,
            SourceSet::PACK | SourceSet::BANK,
        ),
    ];
    let counts = fixture(&[], &[(ITEM, 1), (OTHER, 1), (COINS, 100)], Origin::Session);
    let block = blocked(plan_for(&request(needs), &counts));
    assert_eq!(
        block.need, 0,
        "a bank row cannot satisfy a need that forbids BANK"
    );

    let coin_need = need(
        ITEM,
        1,
        1,
        Presence::Held,
        NeedKind::Required,
        SourceSet::PACK | SourceSet::SHOP,
    );
    let coin_block = AcquireBlock {
        item: Arc::from("coins"),
        need: 0,
        reason: BlockReason::TooFewCoins {
            needed: 50,
            held: 0,
            banked: 100,
        },
        looked: Looked::default(),
        message: Arc::from("coins short"),
    };
    let mut coin_request = request(vec![coin_need]);
    coin_request.coins = CoinPolicy::ForPurchase {
        sources: SourceSet::PACK,
    };
    assert!(!resolved(
        &coin_block,
        &coin_request,
        &counts,
        WorldFacts::default(),
        0
    ));
    coin_request.coins = CoinPolicy::ForPurchase {
        sources: SourceSet::PACK | SourceSet::BANK,
    };
    assert!(resolved(
        &coin_block,
        &coin_request,
        &counts,
        WorldFacts::default(),
        0
    ));
}

#[test]
fn bank_only_coverage_still_uses_the_actual_pack_for_the_withdrawal_target() {
    let counts = fixture(&[(ITEM, 3)], &[(ITEM, 2)], Origin::Session);
    let bank_only = need(
        ITEM,
        2,
        2,
        Presence::Held,
        NeedKind::Required,
        SourceSet::BANK,
    );
    let result = planned(plan_for(&request(vec![bank_only]), &counts));
    assert_eq!(result.withdrawal_count, 1);
    assert_eq!(
        result.withdrawals[0]
            .as_ref()
            .expect("bank-only withdrawal")
            .target,
        5
    );
}

#[cfg(any(unix, windows))]
#[test]
fn bank_observation_clock_formats_local_hours_and_rejects_an_unrepresentable_time() {
    let clock = local_clock(1_700_000_000);
    assert_eq!(clock.len(), 5);
    assert_eq!(&clock[2..3], ":");
    assert!(clock[..2].parse::<u8>().expect("hours") < 24);
    assert!(clock[3..].parse::<u8>().expect("minutes") < 60);
    assert_eq!(local_clock(u64::MAX), "unknown");
}

#[test]
fn planner_accepts_sixty_four_distinct_withdrawals_and_refuses_sixty_five() {
    let rows: Vec<_> = (0..65).map(|offset| (1_000 + offset, 1)).collect();
    let counts = fixture(&[], &rows, Origin::Session);
    let mut planner = Planner::new(&counts, WorldFacts::default());
    for (id, _) in &rows[..64] {
        planner
            .require(&need(
                *id,
                1,
                1,
                Presence::Held,
                NeedKind::Required,
                SourceSet::PACK | SourceSet::BANK,
            ))
            .expect("64 withdrawals fit the bank batch");
    }
    assert_eq!(planner.withdrawal_count(), 64);
    let result = planned(planner.verdict(None));
    assert_eq!(result.withdrawal_count, 64);

    let mut planner = Planner::new(&counts, WorldFacts::default());
    for (id, _) in &rows {
        let result = planner.require(&need(
            *id,
            1,
            1,
            Presence::Held,
            NeedKind::Required,
            SourceSet::PACK | SourceSet::BANK,
        ));
        if *id == 1_064 {
            assert!(matches!(result, Err(ActionError::Unavailable(_))));
        } else {
            result.expect("first 64 distinct withdrawals fit");
        }
    }
}

#[test]
fn synthetic_counts_can_report_unknown_and_session_rows_for_different_ids() {
    let counts = CountsFixture {
        held: &[],
        worn: &[],
        banked: &[(OTHER, 1)],
        unknown_bank_ids: &[ITEM],
        origin: Origin::Session,
        free_slots: Some(28),
    };
    let needs = request(vec![
        need(
            ITEM,
            1,
            1,
            Presence::Held,
            NeedKind::Required,
            SourceSet::PACK | SourceSet::BANK,
        ),
        need(
            OTHER,
            2,
            2,
            Presence::Held,
            NeedKind::Required,
            SourceSet::PACK | SourceSet::BANK,
        ),
    ]);
    let result = planned(plan_for(&needs, &counts));
    assert!(
        result.scan,
        "unknown evidence for one id requires a learning trip"
    );
    assert_eq!(
        result.withdrawal_count, 1,
        "known Session evidence for another id still produces a withdrawal"
    );
    assert_eq!(
        result.withdrawals[0]
            .as_ref()
            .expect("session withdrawal")
            .id,
        OTHER
    );
}
#[test]
fn repeated_id_withdrawals_merge_to_the_largest_final_held_target() {
    let counts = fixture(&[(ITEM, 2)], &[(ITEM, 10)], Origin::Session);
    let mut planner = Planner::new(&counts, WorldFacts::default());
    planner
        .require(&need(
            ITEM,
            5,
            1,
            Presence::Held,
            NeedKind::Required,
            SourceSet::PACK | SourceSet::BANK,
        ))
        .unwrap();
    planner
        .require(&need(
            ITEM,
            8,
            1,
            Presence::Held,
            NeedKind::Required,
            SourceSet::PACK | SourceSet::BANK,
        ))
        .unwrap();
    assert_eq!(planner.withdrawal_count(), 1);
    let result = planned(planner.verdict(None));
    assert_eq!(result.withdrawal_count, 1);
    assert_eq!(
        result.withdrawals[0].as_ref().expect("merged row").target,
        8
    );
}

fn block(reason: BlockReason, need_index: u8) -> AcquireBlock {
    AcquireBlock {
        item: Arc::from("item"),
        need: need_index,
        reason,
        looked: Looked::default(),
        message: Arc::from("blocked"),
    }
}

#[test]
fn resolved_wakes_at_usable_minimum_for_exact_and_acceptable_tiers() {
    let exact_need = need(
        ITEM,
        10,
        5,
        Presence::Held,
        NeedKind::Required,
        SourceSet::PACK | SourceSet::WORN | SourceSet::BANK,
    );
    let req = request(vec![exact_need]);
    let required_block = block(BlockReason::SupplyMissing, 0);
    let held_at_minimum = fixture(&[(ITEM, 5)], &[], Origin::Session);
    assert!(resolved(
        &required_block,
        &req,
        &held_at_minimum,
        WorldFacts::default(),
        0
    ));

    let tier = tier_need(
        ITEM,
        &[LOWER],
        1,
        Presence::Wielded,
        SourceSet::PACK | SourceSet::WORN | SourceSet::BANK,
    );
    let req = request(vec![tier]);
    let tier_block = block(BlockReason::SupplyMissing, 0);
    let held_lower = fixture(&[(LOWER, 1)], &[], Origin::Session);
    assert!(resolved(
        &tier_block,
        &req,
        &held_lower,
        WorldFacts::default(),
        0
    ));
    let worn_lower = CountsFixture {
        held: &[],
        worn: &[(LOWER, 1)],
        banked: &[],
        unknown_bank_ids: &[],
        origin: Origin::Session,
        free_slots: Some(28),
    };
    assert!(resolved(
        &tier_block,
        &req,
        &worn_lower,
        WorldFacts::default(),
        0
    ));
    let banked_lower = fixture(&[], &[(LOWER, 1)], Origin::Session);
    assert!(resolved(
        &tier_block,
        &req,
        &banked_lower,
        WorldFacts::default(),
        0
    ));
}

#[test]
fn resolved_does_not_wake_from_hint_or_unknown_bank_rows() {
    let need = tier_need(
        ITEM,
        &[LOWER],
        1,
        Presence::Carried,
        SourceSet::PACK | SourceSet::WORN | SourceSet::BANK,
    );
    let req = request(vec![need]);
    let block = block(BlockReason::SupplyMissing, 0);
    let hint = fixture(&[], &[(LOWER, 1)], Origin::Hint);
    assert!(!resolved(&block, &req, &hint, WorldFacts::default(), 0));
    let unknown = fixture(&[], &[(LOWER, 1)], Origin::Unknown);
    assert!(!resolved(&block, &req, &unknown, WorldFacts::default(), 0));
}

#[test]
fn resolved_coin_purse_obeys_pack_and_session_bank_permissions() {
    let counts = fixture(&[(COINS, 20)], &[(COINS, 40)], Origin::Session);
    let block = block(
        BlockReason::TooFewCoins {
            needed: 50,
            held: 20,
            banked: 0,
        },
        0,
    );
    let mut req = request(vec![need(
        ITEM,
        1,
        1,
        Presence::Held,
        NeedKind::Required,
        SourceSet::PACK,
    )]);
    req.coins = CoinPolicy::ForPurchase {
        sources: SourceSet::PACK,
    };
    assert!(!resolved(&block, &req, &counts, WorldFacts::default(), 0));
    req.coins = CoinPolicy::ForPurchase {
        sources: SourceSet::PACK | SourceSet::BANK,
    };
    assert!(resolved(&block, &req, &counts, WorldFacts::default(), 0));

    let hint = fixture(&[(COINS, 20)], &[(COINS, 40)], Origin::Hint);
    assert!(!resolved(&block, &req, &hint, WorldFacts::default(), 0));
}

#[test]
fn resolved_no_stock_uses_observed_tick_deadline_and_no_space_uses_free_slots() {
    let req = request(vec![need(
        ITEM,
        1,
        1,
        Presence::Held,
        NeedKind::Required,
        SourceSet::PACK,
    )]);
    let no_stock = block(
        BlockReason::NoStock {
            restock_ticks: 400,
            refused_at_tick: 20,
            aligned_to: None,
            revisit_due: 420,
        },
        0,
    );
    let counts = fixture(&[], &[], Origin::Session);
    assert!(!resolved(
        &no_stock,
        &req,
        &counts,
        WorldFacts::default(),
        419
    ));
    assert!(resolved(
        &no_stock,
        &req,
        &counts,
        WorldFacts::default(),
        420
    ));

    let no_space = block(BlockReason::NoSpace { slots_needed: 2 }, 0);
    let short = CountsFixture {
        free_slots: Some(1),
        ..counts
    };
    let enough = CountsFixture {
        free_slots: Some(2),
        ..counts
    };
    assert!(!resolved(&no_space, &req, &short, WorldFacts::default(), 0));
    assert!(resolved(&no_space, &req, &enough, WorldFacts::default(), 0));
    let unknown_slots = CountsFixture {
        free_slots: None,
        ..counts
    };
    assert!(!resolved(
        &no_space,
        &req,
        &unknown_slots,
        WorldFacts::default(),
        0
    ));
}

#[test]
fn unchanged_resolved_polls_allocate_nothing() {
    let need = tier_need(
        ITEM,
        &[LOWER],
        1,
        Presence::Wielded,
        SourceSet::PACK | SourceSet::WORN | SourceSet::BANK,
    );
    let request = request(vec![need]);
    let block = block(BlockReason::SupplyMissing, 0);
    let counts = fixture(&[], &[], Origin::Session);
    assert!(!resolved(
        &block,
        &request,
        &counts,
        WorldFacts::default(),
        1
    ));
    let allocations = allocation_counter::measure(|| {
        for tick in 1..=1_000 {
            std::hint::black_box(resolved(
                &block,
                &request,
                &counts,
                WorldFacts::default(),
                tick,
            ));
        }
    });
    assert_eq!(allocations, allocation_counter::AllocationInfo::default());
}
