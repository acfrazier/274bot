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
    const RUNE_AXE: i32 = 1359;
    const BRONZE_AXE: i32 = 1351;
    let need = tier_need(
        RUNE_AXE,
        &[BRONZE_AXE],
        1,
        Presence::Wielded,
        SourceSet::PACK | SourceSet::WORN | SourceSet::BANK,
    );
    let counts = fixture(&[], &[(BRONZE_AXE, 1)], Origin::Session);
    let request = request(vec![need]);
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let world = WorldFacts {
        data: Some(&data),
        ..WorldFacts::default()
    };
    let result = planned(plan(&request, &counts, world).unwrap());
    assert_eq!(result.withdrawal_count, 1);
    let row = result.withdrawals[0]
        .as_ref()
        .expect("lower tier withdrawal");
    assert_eq!((row.id, row.target), (BRONZE_AXE, 1));
    assert_eq!(row.name.as_ref(), "Bronze axe");
    assert!(matches!(
        plan(&request, &counts, WorldFacts::default()),
        Err(ActionError::Unavailable(message))
            if message.as_ref() == "selected cache has no name for acquisition alternative 1351"
    ));
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

#[test]
fn bank_observation_text_uses_relative_age_and_keeps_the_unix_time() {
    let observed_at_unix = 1_700_000_000;
    let looked = Looked {
        bank: Some(BankSeen {
            origin: Origin::Session,
            observed_at_unix,
            rows: 12,
            count: 0,
        }),
        ..Looked::default()
    };
    for (elapsed, age) in [
        (0, "less than a minute ago"),
        (59, "less than a minute ago"),
        (60, "1 minute ago"),
        (12 * 60, "12 minutes ago"),
    ] {
        let now =
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(observed_at_unix + elapsed);
        assert_eq!(
            block_message(
                "Harpoon",
                &BlockReason::SupplyMissing,
                &looked,
                "nobody sells it",
                now,
                0,
            ).as_ref(),
            format!("no Harpoon in the pack, worn, or in the bank as seen {age} this session (12 rows); nobody sells it; waiting: put a Harpoon in the pack")
        );
    }
    assert_eq!(looked.bank.unwrap().observed_at_unix, observed_at_unix);

    for observed_at_unix in [observed_at_unix + 1, u64::MAX] {
        let looked = Looked {
            bank: Some(BankSeen {
                observed_at_unix,
                ..looked.bank.unwrap()
            }),
            ..Looked::default()
        };
        let now = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        assert_eq!(
            block_message(
                "Harpoon",
                &BlockReason::SupplyMissing,
                &looked,
                "nobody sells it",
                now,
                0,
            ).as_ref(),
            "no Harpoon in the pack, worn, or in the bank as seen this session (12 rows); nobody sells it; waiting: put a Harpoon in the pack"
        );
    }
}

#[test]
fn saved_hint_block_text_reports_provenance_instead_of_this_session() {
    let observed_at_unix = 1_700_000_000;
    let looked = Looked {
        bank: Some(BankSeen {
            origin: Origin::Hint,
            observed_at_unix,
            rows: 12,
            count: 0,
        }),
        ..Looked::default()
    };
    let now = std::time::UNIX_EPOCH + std::time::Duration::from_secs(observed_at_unix + 12 * 60);
    assert_eq!(
        block_message(
            "Harpoon",
            &BlockReason::SupplyMissing,
            &looked,
            "nobody sells it",
            now,
            0,
        )
        .as_ref(),
        "no Harpoon in the pack, worn, or in the bank as seen 12 minutes ago from a saved hint (12 rows); nobody sells it; waiting: put a Harpoon in the pack"
    );
    let future = Looked {
        bank: Some(BankSeen {
            observed_at_unix: u64::MAX,
            ..looked.bank.unwrap()
        }),
        ..Looked::default()
    };
    assert_eq!(
        block_message(
            "Harpoon",
            &BlockReason::SupplyMissing,
            &future,
            "nobody sells it",
            now,
            0,
        )
        .as_ref(),
        "no Harpoon in the pack, worn, or in the bank as seen from a saved hint (12 rows); nobody sells it; waiting: put a Harpoon in the pack"
    );
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

fn pinned_shop_data() -> Arc<api::game_data::SelectedGameData> {
    api::game_data::for_revision(api::selected::ClientRevision::R289).expect("selected shop data")
}

fn shop_item_id(data: &api::game_data::SelectedGameData, alias: &str) -> i32 {
    data.item_by_alias(alias)
        .unwrap_or_else(|| panic!("missing selected item {alias}"))
        .id
}

fn shop_request(needs: Vec<Need>, vendor: VendorChoice, coin_sources: SourceSet) -> AcquireRequest {
    let mut request = request(needs);
    request.vendor = vendor;
    request.coins = CoinPolicy::ForPurchase {
        sources: coin_sources,
    };
    request
}

fn shop_plan(
    request: &AcquireRequest,
    counts: &dyn Counts,
    data: &api::game_data::SelectedGameData,
    members: bool,
) -> AcquirePlan {
    planned(
        plan(
            request,
            counts,
            WorldFacts {
                data: Some(data),
                members,
            },
        )
        .expect("shop planning"),
    )
}

fn shop_block(
    request: &AcquireRequest,
    counts: &dyn Counts,
    data: &api::game_data::SelectedGameData,
    members: bool,
) -> AcquireBlock {
    blocked(
        plan(
            request,
            counts,
            WorldFacts {
                data: Some(data),
                members,
            },
        )
        .expect("shop planning"),
    )
}

fn purchase_at(plan: &AcquirePlan, index: usize) -> Purchase {
    plan.purchases
        .get(index)
        .copied()
        .flatten()
        .expect("planned purchase")
}

fn withdrawal_target(plan: &AcquirePlan, id: i32) -> Option<i32> {
    plan.withdrawals[..plan.withdrawal_count]
        .iter()
        .flatten()
        .find(|withdrawal| withdrawal.id == id)
        .map(|withdrawal| withdrawal.target)
}

fn two_chunk_fact_quote(fact: &VendorFact<'_>, units: i32) -> i64 {
    shop_facts::quote(fact.record, fact.item, units, fact.item.baseline).saturating_add(
        shop_facts::quote(
            fact.record,
            fact.item,
            units,
            fact.item.baseline.saturating_sub(units),
        ),
    )
}

fn max_shop_quote(
    data: &api::game_data::SelectedGameData,
    item: i32,
    members: bool,
    units: i32,
) -> i64 {
    vendors_for(data, item)
        .filter(|fact| vendor_eligible(fact, members))
        .map(|fact| two_chunk_fact_quote(&fact, units))
        .max()
        .unwrap_or(0)
}

#[test]
fn shop_session_shortage_with_stocked_eligible_vendor_plans_purchase() {
    let data = pinned_shop_data();
    let harpoon = shop_item_id(&data, "harpoon");
    let request = shop_request(
        vec![need(
            harpoon,
            2,
            2,
            Presence::Held,
            NeedKind::Required,
            SourceSet::PACK | SourceSet::BANK | SourceSet::SHOP,
        )],
        VendorChoice::Nearest,
        SourceSet::PACK | SourceSet::BANK,
    );
    let bank = [(harpoon, 0), (COINS, 10_000)];
    let counts = fixture(&[], &bank, Origin::Session);
    let plan = shop_plan(&request, &counts, &data, false);
    let reserve = max_shop_quote(&data, harpoon, false, 2);

    assert_eq!(plan.purchase_count, 1);
    assert_eq!(
        purchase_at(&plan, 0),
        Purchase {
            need: 0,
            item: harpoon,
            target: 2,
            minimum: 2,
        }
    );
    assert_eq!(withdrawal_target(&plan, harpoon), None);
    assert_eq!(withdrawal_target(&plan, COINS), Some(reserve as i32));
}

#[test]
fn exact_harpoon_coin_shortage_keeps_the_typed_park() {
    let data = pinned_shop_data();
    let harpoon = shop_item_id(&data, "harpoon");
    let request = shop_request(
        vec![need(
            harpoon,
            1,
            1,
            Presence::Held,
            NeedKind::Required,
            SourceSet::PACK | SourceSet::SHOP,
        )],
        VendorChoice::Nearest,
        SourceSet::PACK,
    );
    let minimum_quote = minimum_shop_quote(Some(&data), &request.vendor, harpoon, false, 1)
        .expect("pinned harpoon minimum quote");
    let counts = fixture(&[], &[], Origin::Session);

    let block = shop_block(&request, &counts, &data, false);

    assert_eq!(
        block.reason,
        BlockReason::TooFewCoins {
            needed: minimum_quote,
            held: 0,
            banked: 0,
        }
    );
}

#[test]
fn shop_missing_vendor_blocks_and_retains_session_bank_metadata() {
    let data = pinned_shop_data();
    let mithril_axe = shop_item_id(&data, "mithril_axe");
    assert!(vendors_for(&data, mithril_axe).next().is_none());
    let request = shop_request(
        vec![
            need(
                ITEM,
                1,
                1,
                Presence::Held,
                NeedKind::Required,
                SourceSet::PACK,
            ),
            need(
                mithril_axe,
                2,
                2,
                Presence::Held,
                NeedKind::Required,
                SourceSet::PACK | SourceSet::BANK | SourceSet::SHOP,
            ),
        ],
        VendorChoice::Nearest,
        SourceSet::PACK | SourceSet::BANK,
    );
    let bank = [(mithril_axe, 1), (COINS, 1_000)];
    let counts = fixture(&[(ITEM, 1)], &bank, Origin::Session);
    let block = shop_block(&request, &counts, &data, false);

    assert_eq!(block.need, 1);
    assert_eq!(block.reason, BlockReason::NoVendor);
    assert_eq!(
        block.looked.bank,
        Some(BankSeen {
            origin: Origin::Session,
            observed_at_unix: 1_700_000_000,
            rows: 2,
            count: 1,
        })
    );
    assert_eq!(block.looked.pack, 0);
    let unavailable_facts = blocked(
        plan(&request, &counts, WorldFacts::default()).expect("planning without shop facts"),
    );
    assert_eq!(unavailable_facts.reason, BlockReason::NoVendor);
    assert!(unavailable_facts
        .message
        .contains("vendor price unavailable"));
}

#[test]
fn shop_membership_excludes_members_items_on_free_worlds() {
    let data = pinned_shop_data();
    let papyrus = shop_item_id(&data, "papyrus");
    let fact = vendor_by_keeper_config(&data, "aemad", papyrus).expect("pinned papyrus");
    assert!(fact.item.members);
    let request = shop_request(
        vec![need(
            papyrus,
            1,
            1,
            Presence::Held,
            NeedKind::Required,
            SourceSet::PACK | SourceSet::SHOP,
        )],
        VendorChoice::Nearest,
        SourceSet::PACK,
    );
    let counts = fixture(&[(COINS, 1_000)], &[], Origin::Unknown);
    assert_eq!(
        shop_block(&request, &counts, &data, false).reason,
        BlockReason::NoVendor
    );
    assert_eq!(shop_plan(&request, &counts, &data, true).purchase_count, 1);
}

#[test]
fn shop_selected_alternative_is_the_banked_and_purchased_item() {
    let data = pinned_shop_data();
    let steel_axe = shop_item_id(&data, "steel_axe");
    let bronze_axe = shop_item_id(&data, "bronze_axe");
    let request = shop_request(
        vec![tier_need(
            steel_axe,
            &[bronze_axe],
            2,
            Presence::Held,
            SourceSet::PACK | SourceSet::BANK | SourceSet::SHOP,
        )],
        VendorChoice::Explicit {
            keeper_config: Arc::from("bob"),
            anchor: None,
        },
        SourceSet::PACK,
    );
    let bank = [(bronze_axe, 1)];
    let counts = fixture(&[(COINS, 1_000)], &bank, Origin::Session);
    let plan = shop_plan(&request, &counts, &data, false);

    assert_eq!(plan.purchase_count, 1);
    assert_eq!(
        purchase_at(&plan, 0),
        Purchase {
            need: 0,
            item: bronze_axe,
            target: 2,
            minimum: 2,
        }
    );
    assert_eq!(withdrawal_target(&plan, bronze_axe), Some(1));
}

#[test]
fn shop_tier_falls_back_from_unstocked_best_item_to_stocked_lower_tier() {
    let data = pinned_shop_data();
    let mithril_axe = shop_item_id(&data, "mithril_axe");
    let iron_axe = shop_item_id(&data, "iron_axe");
    assert!(vendors_for(&data, mithril_axe).next().is_none());
    assert!(vendors_for(&data, iron_axe).next().is_some());
    let request = shop_request(
        vec![tier_need(
            mithril_axe,
            &[mithril_axe, iron_axe],
            1,
            Presence::Carried,
            SourceSet::PACK | SourceSet::SHOP,
        )],
        VendorChoice::Nearest,
        SourceSet::PACK,
    );
    let counts = fixture(&[(COINS, 1_000)], &[], Origin::Unknown);

    let plan = shop_plan(&request, &counts, &data, false);
    assert_eq!(purchase_at(&plan, 0).item, iron_axe);
}

#[test]
fn shop_tier_skips_an_unaffordable_best_price_for_a_wearable_lower_tier() {
    let data = pinned_shop_data();
    let steel_axe = shop_item_id(&data, "steel_axe");
    let iron_axe = shop_item_id(&data, "iron_axe");
    let steel = vendor_by_keeper_config(&data, "bob", steel_axe).unwrap();
    let iron = vendor_by_keeper_config(&data, "bob", iron_axe).unwrap();
    let steel_price = shop_facts::quote(steel.record, steel.item, 1, steel.item.baseline);
    let iron_price = shop_facts::quote(iron.record, iron.item, 1, iron.item.baseline);
    assert!(iron_price < 100 && steel_price > 100);
    let request = shop_request(
        vec![tier_need(
            steel_axe,
            &[steel_axe, iron_axe],
            1,
            Presence::Wielded,
            SourceSet::PACK | SourceSet::SHOP,
        )],
        VendorChoice::Nearest,
        SourceSet::PACK,
    );
    let counts = fixture(&[(COINS, 100)], &[], Origin::Unknown);

    let plan = shop_plan(&request, &counts, &data, false);
    assert_eq!(purchase_at(&plan, 0).item, iron_axe);
    assert_eq!(
        plan.wear_count, 0,
        "the selected purchased tier is worn after routing"
    );
}

#[test]
fn shop_tier_filters_unwieldable_items_and_existing_tiers_still_win() {
    let data = pinned_shop_data();
    let rune_axe = shop_item_id(&data, "rune_axe");
    let iron_axe = shop_item_id(&data, "iron_axe");
    let bronze_axe = shop_item_id(&data, "bronze_axe");
    let request = shop_request(
        vec![tier_need(
            iron_axe,
            &[rune_axe, iron_axe, bronze_axe],
            1,
            Presence::Wielded,
            SourceSet::PACK | SourceSet::WORN | SourceSet::BANK | SourceSet::SHOP,
        )],
        VendorChoice::Nearest,
        SourceSet::PACK | SourceSet::BANK,
    );
    let coins = [(COINS, 1_000)];
    let held_rows = [(rune_axe, 1), (COINS, 1_000)];
    let rune_rows = [(rune_axe, 1)];
    let no_tools = fixture(&coins, &[], Origin::Session);
    let purchase_plan = shop_plan(&request, &no_tools, &data, false);
    assert_eq!(purchase_at(&purchase_plan, 0).item, iron_axe);
    assert_eq!(purchase_plan.wear_count, 0);

    let held_rune = fixture(&held_rows, &[], Origin::Unknown);
    assert!(matches!(
        plan(
            &request,
            &held_rune,
            WorldFacts {
                data: Some(&data),
                members: false,
            }
        )
        .unwrap(),
        AcquireVerdict::Satisfied
    ));

    let worn_rune = CountsFixture {
        held: &coins,
        worn: &rune_rows,
        banked: &[],
        unknown_bank_ids: &[],
        origin: Origin::Unknown,
        free_slots: Some(28),
    };
    assert!(matches!(
        plan(
            &request,
            &worn_rune,
            WorldFacts {
                data: Some(&data),
                members: false,
            }
        )
        .unwrap(),
        AcquireVerdict::Satisfied
    ));

    let banked_rune = fixture(&coins, &rune_rows, Origin::Session);
    let plan = shop_plan(&request, &banked_rune, &data, false);
    assert_eq!(plan.purchase_count, 0);
    assert_eq!(plan.wear_count, 0);
    assert_eq!(withdrawal_target(&plan, rune_axe), Some(1));
}

#[test]
fn shop_tier_unknown_coin_bank_scans_before_skipping_the_best_price() {
    let data = pinned_shop_data();
    let steel_axe = shop_item_id(&data, "steel_axe");
    let iron_axe = shop_item_id(&data, "iron_axe");
    let request = shop_request(
        vec![tier_need(
            steel_axe,
            &[steel_axe, iron_axe],
            1,
            Presence::Wielded,
            SourceSet::PACK | SourceSet::SHOP,
        )],
        VendorChoice::Nearest,
        SourceSet::PACK | SourceSet::BANK,
    );
    let counts = fixture(&[(COINS, 1)], &[], Origin::Unknown);

    let world = WorldFacts {
        data: Some(&data),
        members: false,
    };
    let mut planner = Planner::new(&counts, world);
    planner
        .require_with_purchase(&request.needs[0], &request.vendor, request.coins, world)
        .unwrap();
    assert_eq!(planner.scan().map(|detail| detail.id), Some(steel_axe));
    let acquire_plan = planned(planner.verdict(None));
    assert!(acquire_plan.scan);
    assert_eq!(acquire_plan.purchase_count, 0);
}

#[test]
fn tiered_no_vendor_detail_names_the_best_tier_and_failed_condition() {
    let data = pinned_shop_data();
    let mithril_axe = shop_item_id(&data, "mithril_axe");
    let mut need = tier_need(
        mithril_axe,
        &[mithril_axe],
        1,
        Presence::Carried,
        SourceSet::PACK | SourceSet::SHOP,
    );
    need.name = Arc::from("Mithril axe");
    let request = shop_request(vec![need], VendorChoice::Nearest, SourceSet::PACK);
    let counts = fixture(&[(COINS, 1_000)], &[], Origin::Unknown);

    let block = shop_block(&request, &counts, &data, false);
    assert_eq!(block.reason, BlockReason::NoVendor);
    assert!(block.message.contains("best tier Mithril axe"));
    assert!(block.message.contains("no eligible vendor stocks it"));
}

#[test]
fn shop_purchase_units_exclude_pack_worn_and_planned_bank_take() {
    let data = pinned_shop_data();
    let bronze_axe = shop_item_id(&data, "bronze_axe");
    let request = shop_request(
        vec![need(
            bronze_axe,
            8,
            5,
            Presence::Wielded,
            NeedKind::Required,
            SourceSet::PACK | SourceSet::WORN | SourceSet::BANK | SourceSet::SHOP,
        )],
        VendorChoice::Explicit {
            keeper_config: Arc::from("bob"),
            anchor: None,
        },
        SourceSet::PACK,
    );
    let counts = CountsFixture {
        held: &[(bronze_axe, 2), (COINS, 1_000)],
        worn: &[(bronze_axe, 1)],
        banked: &[(bronze_axe, 2)],
        unknown_bank_ids: &[],
        origin: Origin::Session,
        free_slots: Some(28),
    };
    let plan = shop_plan(&request, &counts, &data, false);

    assert_eq!(plan.purchase_count, 1);
    assert_eq!(
        purchase_at(&plan, 0),
        Purchase {
            need: 0,
            item: bronze_axe,
            target: 7,
            minimum: 4,
        }
    );
    assert_eq!(withdrawal_target(&plan, bronze_axe), Some(4));
}

#[test]
fn shop_reserve_sums_the_maximum_quote_for_two_needs() {
    let data = pinned_shop_data();
    let iron_axe = shop_item_id(&data, "iron_axe");
    let bronze_pickaxe = shop_item_id(&data, "bronze_pickaxe");
    let request = shop_request(
        vec![
            need(
                iron_axe,
                2,
                2,
                Presence::Held,
                NeedKind::Required,
                SourceSet::PACK | SourceSet::SHOP,
            ),
            need(
                bronze_pickaxe,
                2,
                2,
                Presence::Held,
                NeedKind::Required,
                SourceSet::PACK | SourceSet::SHOP,
            ),
        ],
        VendorChoice::Nearest,
        SourceSet::PACK | SourceSet::BANK,
    );
    let counts = fixture(&[], &[(COINS, 1_000_000)], Origin::Session);
    let plan = shop_plan(&request, &counts, &data, false);
    let expected =
        max_shop_quote(&data, iron_axe, false, 2) + max_shop_quote(&data, bronze_pickaxe, false, 2);

    assert_eq!(plan.purchase_count, 2);
    assert_eq!(withdrawal_target(&plan, COINS), Some(expected as i32));
}

#[test]
fn shop_reserve_is_exact_two_chunk_quote_and_surplus_is_below_baseline() {
    let data = pinned_shop_data();
    let bronze_axe = shop_item_id(&data, "bronze_axe");
    let fact =
        vendor_by_keeper_config(&data, "bob", bronze_axe).expect("pinned Bob bronze axe stock");
    let units = 3;
    let baseline = fact.item.baseline;
    let baseline_quote = shop_facts::quote(fact.record, fact.item, units, baseline);
    let expected = baseline_quote
        + shop_facts::quote(
            fact.record,
            fact.item,
            units,
            baseline.saturating_sub(units),
        );
    let surplus_quote = shop_facts::quote(
        fact.record,
        fact.item,
        units,
        baseline.saturating_add(units),
    );
    assert!(surplus_quote < baseline_quote);
    let request = shop_request(
        vec![need(
            bronze_axe,
            units,
            units,
            Presence::Held,
            NeedKind::Required,
            SourceSet::PACK | SourceSet::SHOP,
        )],
        VendorChoice::Explicit {
            keeper_config: Arc::from("bob"),
            anchor: None,
        },
        SourceSet::PACK | SourceSet::BANK,
    );
    let counts = fixture(&[], &[(COINS, 10_000)], Origin::Session);
    let plan = shop_plan(&request, &counts, &data, false);

    assert_eq!(withdrawal_target(&plan, COINS), Some(expected as i32));
}

#[test]
fn shop_pack_only_coin_policy_never_withdraws_coins() {
    let data = pinned_shop_data();
    let vial_water = shop_item_id(&data, "vial_water");
    let request = shop_request(
        vec![need(
            vial_water,
            1,
            1,
            Presence::Held,
            NeedKind::Required,
            SourceSet::PACK | SourceSet::SHOP,
        )],
        VendorChoice::Explicit {
            keeper_config: Arc::from("aemad"),
            anchor: None,
        },
        SourceSet::PACK,
    );
    let minimum_quote = minimum_shop_quote(Some(&data), &request.vendor, vial_water, false, 1)
        .expect("pinned Aemad vial water minimum quote");
    let held_coins = [(COINS, minimum_quote as i32)];
    let counts = fixture(&held_coins, &[(COINS, 1_000)], Origin::Session);
    let plan = shop_plan(&request, &counts, &data, false);

    assert_eq!(plan.purchase_count, 1);
    assert_eq!(withdrawal_target(&plan, COINS), None);
}

#[test]
fn shop_reserve_caps_held_target_and_merges_optional_coin_target() {
    let data = pinned_shop_data();
    let bronze_axe = shop_item_id(&data, "bronze_axe");
    let fact =
        vendor_by_keeper_config(&data, "bob", bronze_axe).expect("pinned Bob bronze axe stock");
    let reserve = two_chunk_fact_quote(&fact, 1);
    let purchase_need = need(
        bronze_axe,
        1,
        1,
        Presence::Held,
        NeedKind::Required,
        SourceSet::PACK | SourceSet::SHOP,
    );
    let request = shop_request(
        vec![purchase_need.clone()],
        VendorChoice::Explicit {
            keeper_config: Arc::from("bob"),
            anchor: None,
        },
        SourceSet::PACK | SourceSet::BANK,
    );
    let minimum_quote = minimum_shop_quote(Some(&data), &request.vendor, bronze_axe, false, 1)
        .expect("pinned Bob bronze axe minimum quote");
    assert!(reserve > minimum_quote);
    let held = 2;
    let banked = minimum_quote as i32 - held;
    let held_coins = [(COINS, held)];
    let bank_coins = [(COINS, banked)];
    let limited = fixture(&held_coins, &bank_coins, Origin::Session);
    let plan = shop_plan(&request, &limited, &data, false);
    assert_eq!(withdrawal_target(&plan, COINS), Some(held + banked));

    let coin_target = need(
        COINS,
        100,
        0,
        Presence::Held,
        NeedKind::Optional,
        SourceSet::PACK | SourceSet::BANK,
    );
    let request = shop_request(
        vec![purchase_need, coin_target],
        VendorChoice::Explicit {
            keeper_config: Arc::from("bob"),
            anchor: None,
        },
        SourceSet::PACK | SourceSet::BANK,
    );
    let counts = fixture(&held_coins, &[(COINS, 1_000)], Origin::Session);
    let plan = shop_plan(&request, &counts, &data, false);
    assert!(reserve < 100);
    assert_eq!(withdrawal_target(&plan, COINS), Some(100));
}

#[test]
fn shop_uses_content_price_floor_before_walking_but_not_the_full_reserve() {
    let data = pinned_shop_data();
    let bait = shop_item_id(&data, "fishing_bait");
    let short_request = shop_request(
        vec![need(
            bait,
            2,
            2,
            Presence::Held,
            NeedKind::Required,
            SourceSet::PACK | SourceSet::SHOP,
        )],
        VendorChoice::Explicit {
            keeper_config: Arc::from("gerrant"),
            anchor: None,
        },
        SourceSet::PACK | SourceSet::BANK,
    );
    let minimum_quote = minimum_shop_quote(Some(&data), &short_request.vendor, bait, false, 2)
        .expect("pinned Gerrant minimum quote");
    assert!(
        minimum_quote > 2,
        "the two-bait quote must exceed the 1 gp-per-bait floor"
    );

    let insufficient_coins = fixture(&[(COINS, 2)], &[(COINS, 0)], Origin::Session);
    let world = WorldFacts {
        data: Some(&data),
        members: false,
    };
    let mut planner = Planner::new(&insufficient_coins, world);
    planner
        .require_with_purchase(
            &short_request.needs[0],
            &short_request.vendor,
            short_request.coins,
            world,
        )
        .expect("shop planning");
    assert_eq!(
        planner.plan.purchase_count, 0,
        "insufficient coins must block before a purchase is created"
    );
    let block = blocked(planner.verdict(None));
    assert_eq!(
        block.reason,
        BlockReason::TooFewCoins {
            needed: minimum_quote,
            held: 2,
            banked: 0,
        }
    );

    let gerrant =
        vendor_by_keeper_config(&data, "gerrant", bait).expect("pinned Gerrant fishing bait stock");
    let reserve = two_chunk_fact_quote(&gerrant, 2);
    assert!(reserve > minimum_quote);
    let coins = [(COINS, minimum_quote as i32)];
    let enough_for_minimum = fixture(&coins, &[(COINS, 0)], Origin::Session);
    let plan = shop_plan(&short_request, &enough_for_minimum, &data, false);
    assert_eq!(plan.purchase_count, 1);
    assert_eq!(withdrawal_target(&plan, COINS), None);
}

#[test]
fn shop_unknown_and_hint_coin_banks_request_verification() {
    let data = pinned_shop_data();
    let vial_water = shop_item_id(&data, "vial_water");
    let request = shop_request(
        vec![need(
            vial_water,
            1,
            1,
            Presence::Held,
            NeedKind::Required,
            SourceSet::PACK | SourceSet::SHOP,
        )],
        VendorChoice::Explicit {
            keeper_config: Arc::from("aemad"),
            anchor: None,
        },
        SourceSet::PACK | SourceSet::BANK,
    );

    let minimum_quote = minimum_shop_quote(Some(&data), &request.vendor, vial_water, false, 1)
        .expect("pinned Aemad vial water minimum quote");
    let held_coins = [(COINS, minimum_quote as i32)];
    for origin in [Origin::Unknown, Origin::Hint] {
        let enough_for_floor = fixture(&held_coins, &[(COINS, 100)], origin);
        let plan = shop_plan(&request, &enough_for_floor, &data, false);
        assert!(plan.scan, "{origin:?}");
        assert_eq!(plan.purchase_count, 1, "{origin:?}");
        assert_eq!(withdrawal_target(&plan, COINS), None, "{origin:?}");

        let below_floor = fixture(&[], &[(COINS, 100)], origin);
        let plan = shop_plan(&request, &below_floor, &data, false);
        assert!(plan.scan, "{origin:?}");
        assert_eq!(plan.purchase_count, 0, "{origin:?}");
    }
}

#[test]
fn nearest_reserve_covers_every_vendor_then_clamps_to_held_plus_bank() {
    let data = pinned_shop_data();
    let iron_axe = shop_item_id(&data, "iron_axe");
    let vendors: Vec<_> = vendors_for(&data, iron_axe)
        .filter(|fact| vendor_eligible(fact, false))
        .collect();
    assert!(vendors.len() > 1);
    let quotes: Vec<_> = vendors
        .iter()
        .map(|fact| two_chunk_fact_quote(fact, 1))
        .collect();
    let maximum = *quotes.iter().max().expect("eligible vendor quote");
    assert!(quotes.iter().all(|quote| *quote <= maximum));
    let request = shop_request(
        vec![need(
            iron_axe,
            1,
            1,
            Presence::Held,
            NeedKind::Required,
            SourceSet::PACK | SourceSet::SHOP,
        )],
        VendorChoice::Nearest,
        SourceSet::PACK | SourceSet::BANK,
    );
    let bank = [(COINS, maximum as i32 + 100)];
    let enough = fixture(&[], &bank, Origin::Session);
    let plan = shop_plan(&request, &enough, &data, false);
    assert_eq!(withdrawal_target(&plan, COINS), Some(maximum as i32));
    for quote in &quotes {
        assert!(*quote <= i64::from(withdrawal_target(&plan, COINS).unwrap()));
    }

    let minimum_quote = minimum_shop_quote(Some(&data), &request.vendor, iron_axe, false, 1)
        .expect("minimum quote across eligible iron axe vendors");
    let held = 3;
    let banked = minimum_quote as i32 - held;
    let held_coins = [(COINS, held)];
    let bank_coins = [(COINS, banked)];
    let capped = fixture(&held_coins, &bank_coins, Origin::Session);
    let plan = shop_plan(&request, &capped, &data, false);
    assert!(maximum > minimum_quote);
    assert_eq!(withdrawal_target(&plan, COINS), Some(held + banked));
}

#[test]
fn nearest_pre_walk_message_names_all_eligible_vendors_and_price_range() {
    let data = pinned_shop_data();
    let iron_axe = shop_item_id(&data, "iron_axe");
    let vendors: Vec<_> = vendors_for(&data, iron_axe)
        .filter(|fact| vendor_eligible(fact, false))
        .collect();
    assert!(vendors.len() > 1);
    let names = vendors
        .iter()
        .map(|fact| fact.keeper_name)
        .collect::<Vec<_>>()
        .join(" and ");
    let mut prices = vendors
        .iter()
        .map(|fact| shop_facts::quote(fact.record, fact.item, 1, fact.item.baseline));
    let first = prices.next().expect("vendor price");
    let (low, high) = prices.fold((first, first), |(low, high), price| {
        (low.min(price), high.max(price))
    });
    let range = if low == high {
        low.to_string()
    } else {
        format!("{low}-{high}")
    };
    let expected = format!(
        "eligible vendors {names} sell one for {range} gp at baseline stock; you hold 1 gp and the bank holds 2 gp"
    );

    assert_eq!(
        vendor_line_for_purchase(Some(&data), &VendorChoice::Nearest, iron_axe, false, 1, 2,),
        expected
    );
}

#[test]
fn block_messages_for_every_vendor_reason_are_byte_exact() {
    let looked = Looked {
        bank: Some(BankSeen {
            origin: Origin::Session,
            observed_at_unix: 1_700_000_000,
            rows: 37,
            count: 0,
        }),
        ..Looked::default()
    };
    let now = || std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_060);
    let prefix =
        "no Harpoon in the pack, worn, or in the bank as seen 1 minute ago this session (37 rows); ";
    let missing = BlockReason::SupplyMissing;
    assert_eq!(
        block_message("Harpoon", &missing, &looked, "nobody sells it", now(), 60).as_ref(),
        format!("{prefix}nobody sells it; waiting: put a Harpoon in the pack")
    );
    let no_vendor = BlockReason::NoVendor;
    assert_eq!(
        block_message("Harpoon", &no_vendor, &looked, "nobody sells it", now(), 60).as_ref(),
        format!("{prefix}nobody sells it; waiting: put a Harpoon in the pack")
    );
    let too_few = BlockReason::TooFewCoins {
        needed: 1,
        held: 1,
        banked: 2,
    };
    let data = pinned_shop_data();
    let harpoon = shop_item_id(&data, "harpoon");
    let vendor_line =
        vendor_line_for_purchase(Some(&data), &VendorChoice::Nearest, harpoon, false, 1, 2);
    assert_eq!(
        block_message("Harpoon", &too_few, &looked, &vendor_line, now(), 60,).as_ref(),
        format!("{prefix}{vendor_line}; waiting: put a Harpoon or coins in the pack")
    );
    let unreachable = BlockReason::VendorUnreachable;
    assert_eq!(
        block_message(
            "Harpoon",
            &unreachable,
            &looked,
            "Gerrant and Harry cannot be reached from here",
            now(),
            60,
        )
        .as_ref(),
        format!(
            "{prefix}Gerrant and Harry cannot be reached from here; waiting: Stop, move closer, then Start"
        )
    );
    let undecided = BlockReason::VendorPickUndecided;
    assert_eq!(
        block_message(
            "Harpoon",
            &undecided,
            &looked,
            "the route search for Gerrant did not finish",
            now(),
            60,
        )
        .as_ref(),
        format!(
            "{prefix}the route search for Gerrant did not finish; waiting: Stop, allow the route in the card, then Start"
        )
    );
    let no_stock = BlockReason::NoStock {
        restock_ticks: 400,
        refused_at_tick: 20,
        aligned_to: None,
        revisit_due: 90,
    };
    assert_eq!(
        block_message(
            "Harpoon",
            &no_stock,
            &looked,
            "Gerrant had none (seen now)",
            now(),
            60,
        )
        .as_ref(),
        format!(
            "{prefix}Gerrant had none (seen now); waiting: the shop restocks one every 400 ticks, revisit in 30 ticks"
        )
    );
    let no_space = BlockReason::NoSpace { slots_needed: 2 };
    assert_eq!(
        block_message(
            "Harpoon",
            &no_space,
            &looked,
            "Gerrant sells one for 5 gp at baseline stock; you hold 0 gp and the bank holds 0 gp",
            now(),
            60,
        )
        .as_ref(),
        format!(
            "{prefix}Gerrant sells one for 5 gp at baseline stock; you hold 0 gp and the bank holds 0 gp; waiting: free 2 pack slots"
        )
    );
}

#[test]
fn shop_purchase_marks_unworn_wielded_item_for_wear() {
    let data = pinned_shop_data();
    let bronze_axe = shop_item_id(&data, "bronze_axe");
    let request = shop_request(
        vec![need(
            bronze_axe,
            1,
            1,
            Presence::Wielded,
            NeedKind::Required,
            SourceSet::PACK | SourceSet::SHOP,
        )],
        VendorChoice::Explicit {
            keeper_config: Arc::from("bob"),
            anchor: None,
        },
        SourceSet::PACK,
    );
    let counts = fixture(&[(COINS, 1_000)], &[], Origin::Unknown);
    let plan = shop_plan(&request, &counts, &data, false);

    assert_eq!(plan.purchase_count, 1);
    assert_eq!(plan.wear_count, 1);
    assert_eq!(plan.wears[0], Some(bronze_axe));
}
