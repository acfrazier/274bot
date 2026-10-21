use super::*;

fn run() -> RunKey {
    RunKey {
        slot: 13,
        run: 28,
        session: 0,
    }
}

fn fixture_dispatch() -> Dispatch {
    Dispatch {
        run: run(),
        tick: 10,
        held_before: 20,
        chunk: 5,
        chat_seq: 100,
        closes_at: 7,
        stock_before: 12,
        refusal: None,
        resolved: false,
    }
}

fn observation(
    tick: u64,
    held: i32,
    refusal: Option<(i32, PurchaseRefusal)>,
) -> PurchaseObservation {
    PurchaseObservation {
        run: run(),
        tick,
        held,
        refusal: refusal.map(|(sequence, kind)| RefusalLine { sequence, kind }),
    }
}

#[test]
fn refusal_parser_accepts_only_exact_shop_lines() {
    assert_eq!(
        PurchaseRefusal::from_line("You don't have enough coins."),
        Some(PurchaseRefusal::Coins)
    );
    assert_eq!(
        PurchaseRefusal::from_line("The shop has run out of stock."),
        Some(PurchaseRefusal::Stock)
    );
    assert_eq!(
        PurchaseRefusal::from_line("You don't have enough inventory space."),
        Some(PurchaseRefusal::Space)
    );
    assert_eq!(
        PurchaseRefusal::from_line("A player says: You don't have enough coins."),
        None
    );
    assert_eq!(
        PurchaseRefusal::from_line("You don't have enough coins!"),
        None
    );
}

#[test]
fn full_chunk_transfer_resolves_on_first_observed_edge() {
    let mut dispatch = fixture_dispatch();
    let settled = dispatch
        .observe(observation(10, 25, None), SettlementWitness::Content)
        .unwrap();
    assert_eq!(
        settled,
        Some(ResolvedDispatch {
            landed: 5,
            refusal: None,
            unanswered: false
        })
    );
    assert_eq!(
        dispatch
            .observe(observation(10, 25, None), SettlementWitness::FreshPage)
            .unwrap(),
        None
    );
}

#[test]
fn chat_refusal_waits_for_later_partial_pack_evidence() {
    let mut dispatch = fixture_dispatch();
    assert_eq!(
        dispatch
            .observe(
                observation(11, 20, Some((101, PurchaseRefusal::Coins))),
                SettlementWitness::Content
            )
            .unwrap(),
        None
    );
    assert_eq!(
        dispatch
            .observe(observation(12, 23, None), SettlementWitness::Content)
            .unwrap(),
        Some(ResolvedDispatch {
            landed: 3,
            refusal: Some(PurchaseRefusal::Coins),
            unanswered: false,
        })
    );
}

#[test]
fn partial_pack_before_refusal_does_not_settle_on_repeated_snapshot() {
    let mut dispatch = fixture_dispatch();
    assert_eq!(
        dispatch
            .observe(observation(11, 23, None), SettlementWitness::Content)
            .unwrap(),
        None
    );
    assert_eq!(
        dispatch
            .observe(
                observation(12, 23, Some((101, PurchaseRefusal::Space))),
                SettlementWitness::Content
            )
            .unwrap(),
        None
    );
    assert_eq!(
        dispatch
            .observe(observation(12, 23, None), SettlementWitness::Content)
            .unwrap(),
        None
    );
    assert_eq!(
        dispatch
            .observe(observation(14, 23, None), SettlementWitness::Content)
            .unwrap(),
        Some(ResolvedDispatch {
            landed: 3,
            refusal: Some(PurchaseRefusal::Space),
            unanswered: false,
        })
    );
}

#[test]
fn separated_line_edge_and_pack_settle_partial_refusal() {
    let mut dispatch = fixture_dispatch();
    assert_eq!(
        dispatch
            .observe(
                observation(11, 20, Some((101, PurchaseRefusal::Stock))),
                SettlementWitness::Content
            )
            .unwrap(),
        None
    );
    assert_eq!(
        dispatch
            .observe(observation(12, 20, None), SettlementWitness::Content)
            .unwrap(),
        None
    );
    assert_eq!(
        dispatch
            .observe(observation(13, 23, None), SettlementWitness::Content)
            .unwrap(),
        Some(ResolvedDispatch {
            landed: 3,
            refusal: Some(PurchaseRefusal::Stock),
            unanswered: false,
        })
    );
}

#[test]
fn partial_three_of_five_and_coins_refusal_settle_in_either_order() {
    let mut chat_first = fixture_dispatch();
    assert_eq!(
        chat_first
            .observe(
                observation(11, 20, Some((101, PurchaseRefusal::Coins))),
                SettlementWitness::Content
            )
            .unwrap(),
        None
    );
    assert_eq!(
        chat_first
            .observe(observation(12, 23, None), SettlementWitness::Content)
            .unwrap()
            .unwrap()
            .landed,
        3
    );

    let mut pack_first = fixture_dispatch();
    assert_eq!(
        pack_first
            .observe(observation(11, 23, None), SettlementWitness::Content)
            .unwrap(),
        None
    );
    assert_eq!(
        pack_first
            .observe(
                observation(12, 23, Some((101, PurchaseRefusal::Coins))),
                SettlementWitness::Content
            )
            .unwrap(),
        None
    );
    assert_eq!(
        pack_first
            .observe(observation(14, 23, None), SettlementWitness::Content)
            .unwrap()
            .unwrap()
            .landed,
        3
    );
}

#[test]
fn refusal_line_is_counted_once_and_stale_lines_are_ignored() {
    let mut dispatch = fixture_dispatch();
    assert_eq!(
        dispatch
            .observe(
                observation(11, 20, Some((100, PurchaseRefusal::Coins))),
                SettlementWitness::Content
            )
            .unwrap(),
        None
    );
    assert!(
        dispatch.refusal.is_none(),
        "a refusal at the observed chat watermark is stale"
    );
    assert_eq!(
        dispatch
            .observe(
                observation(11, 20, Some((101, PurchaseRefusal::Coins))),
                SettlementWitness::Content
            )
            .unwrap(),
        None
    );
    for _ in 0..4 {
        assert_eq!(
            dispatch
                .observe(
                    observation(11, 20, Some((101, PurchaseRefusal::Coins))),
                    SettlementWitness::Content
                )
                .unwrap(),
            None
        );
    }
    assert_eq!(dispatch.refusal.unwrap().tick, 11);
    assert_eq!(dispatch.refusal.unwrap().held, 20);
}

#[test]
fn first_refusal_stamp_stays_sticky_until_two_later_edges() {
    let mut dispatch = fixture_dispatch();
    assert_eq!(
        dispatch
            .observe(
                observation(11, 20, Some((101, PurchaseRefusal::Coins))),
                SettlementWitness::Content
            )
            .unwrap(),
        None
    );
    assert_eq!(
        dispatch
            .observe(
                observation(11, 20, Some((102, PurchaseRefusal::Stock))),
                SettlementWitness::Content
            )
            .unwrap(),
        None
    );
    assert_eq!(dispatch.refusal.unwrap().kind, PurchaseRefusal::Coins);
    assert_eq!(
        dispatch
            .observe(observation(12, 20, None), SettlementWitness::Content)
            .unwrap(),
        None
    );
    assert_eq!(
        dispatch
            .observe(observation(13, 20, None), SettlementWitness::Content)
            .unwrap()
            .unwrap()
            .refusal,
        Some(PurchaseRefusal::Coins)
    );

    let mut no_tick_advance = fixture_dispatch();
    assert_eq!(
        no_tick_advance
            .observe(
                observation(11, 20, Some((101, PurchaseRefusal::Coins))),
                SettlementWitness::Content
            )
            .unwrap(),
        None
    );
    for _ in 0..4 {
        assert_eq!(
            no_tick_advance
                .observe(observation(11, 20, None), SettlementWitness::Content)
                .unwrap(),
            None
        );
    }
}

#[test]
fn foreign_run_is_stale_and_does_not_account_the_dispatch() {
    let mut dispatch = fixture_dispatch();
    let mut foreign = observation(12, 25, None);
    foreign.run.run += 1;
    assert!(matches!(
        dispatch.observe(foreign, SettlementWitness::FreshPage),
        Err(crate::native::ActionError::Stale)
    ));
    assert!(!dispatch.resolved);
}

#[test]
fn close_observations_request_roundtrip_but_never_settle_content() {
    let mut dispatch = fixture_dispatch();
    assert!(
        dispatch.roundtrip_due(10, 8, true),
        "a new close requests the ordered same-keeper Trade"
    );
    assert!(
        dispatch.roundtrip_due(10, 7, false),
        "a locally absent page requests the ordered same-keeper Trade"
    );
    assert_eq!(
        dispatch
            .observe(observation(10, 20, None), SettlementWitness::Content)
            .unwrap(),
        None
    );
    assert!(!dispatch.resolved);
}

#[test]
fn timeout_requests_roundtrip_and_late_full_chunk_resolves_only_once() {
    let mut dispatch = fixture_dispatch();
    assert!(dispatch.roundtrip_due(12, 7, true));
    assert_eq!(
        dispatch
            .observe(observation(13, 25, None), SettlementWitness::Content)
            .unwrap()
            .unwrap()
            .landed,
        5
    );
    assert_eq!(
        dispatch
            .observe(observation(14, 25, None), SettlementWitness::FreshPage)
            .unwrap(),
        None
    );
}

#[test]
fn fresh_page_settles_partial_refusal_and_zero_landing_outcomes_once() {
    let mut partial = fixture_dispatch();
    assert_eq!(
        partial
            .observe(
                observation(11, 23, Some((101, PurchaseRefusal::Coins))),
                SettlementWitness::FreshPage
            )
            .unwrap(),
        Some(ResolvedDispatch {
            landed: 3,
            refusal: Some(PurchaseRefusal::Coins),
            unanswered: false,
        })
    );
    assert_eq!(
        partial
            .observe(observation(12, 25, None), SettlementWitness::FreshPage)
            .unwrap(),
        None
    );

    let mut late_refusal = fixture_dispatch();
    assert_eq!(
        late_refusal
            .observe(
                observation(11, 20, Some((101, PurchaseRefusal::Stock))),
                SettlementWitness::FreshPage
            )
            .unwrap(),
        Some(ResolvedDispatch {
            landed: 0,
            refusal: Some(PurchaseRefusal::Stock),
            unanswered: false,
        })
    );

    let mut no_answer = fixture_dispatch();
    assert_eq!(
        no_answer
            .observe(observation(11, 20, None), SettlementWitness::FreshPage)
            .unwrap(),
        Some(ResolvedDispatch {
            landed: 0,
            refusal: None,
            unanswered: true,
        })
    );
}

#[test]
fn close_wait_requires_two_elapsed_counts_and_rejects_a_missing_close() {
    let mut close = CloseWait::new(7, 10);
    assert!(!close.observe(7, 10).unwrap(), "no interval has elapsed");
    assert!(
        !close.observe(7, 12).unwrap(),
        "the first elapsed count starts the second interval"
    );
    assert!(close.second_count);
    assert!(
        !close.observe(7, 12).unwrap(),
        "an unchanged tick does not complete the second interval"
    );
    assert!(matches!(
        close.observe(7, 14),
        Err(crate::native::ActionError::Failed(_))
    ));

    let mut closed_after_page_absence = CloseWait::new(7, 10);
    assert!(
        closed_after_page_absence.observe(8, 10).unwrap(),
        "a later close count confirms completion even when the local page is absent"
    );
}
