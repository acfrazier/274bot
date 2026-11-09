//! The Gatherer's bank trip under survival interrupts, relogs and access
//! recovery, driven through the acquisition run that owns the trip's bank
//! work. Each test is a port of the same-named test that drove the Gatherer's
//! own step machine (compat finish): the assertions are the same, the trip is
//! the shared `AcquireRun`.
use super::tests::{accept_last_interaction, antipoison_item, survival_condition, BaitFixture};
use super::*;
use crate::acquire::{Need, NeedKind, Presence, SourceSet};
use crate::quester::families::tests::{def, with_tick};
use api::condition::{FoodSummary, PoisonState};
use api::snapshot::{ItemActionFamily, ItemContainer};

fn clear() -> api::condition::BotCondition {
    survival_condition(PoisonState::Clear, 60, FoodSummary::default())
}

fn onset() -> api::condition::BotCondition {
    survival_condition(
        PoisonState::Onset {
            since: 20,
            q: Some(11),
            hit_since_line: false,
        },
        5,
        FoodSummary::default(),
    )
}

fn bank_item(id: i32, name: &str, count: i32, slot: i32) -> ItemView {
    ItemView {
        def: def(id, name),
        container: ItemContainer::Bank,
        action_family: ItemActionFamily::Component,
        slot,
        count,
        actions: vec![Some("Withdraw-1".into()), Some("Withdraw-X".into())],
        component_id: 7,
    }
}

fn held(id: i32, name: &str, count: i32, slot: i32) -> ItemView {
    ItemView {
        def: def(id, name),
        container: ItemContainer::Inventory,
        action_family: ItemActionFamily::Held,
        slot,
        count,
        actions: Vec::new(),
        component_id: 3214,
    }
}

/// Bait 2 and Feather 1, the two bank rows the withdraw tests ask for.
fn withdraw_needs() -> Arc<[Need]> {
    let need = |item, name: &str, target| Need {
        item,
        name: Arc::from(name),
        target,
        minimum: 1,
        presence: Presence::Held,
        kind: NeedKind::Required,
        sources: SourceSet::PACK | SourceSet::BANK,
        alternatives: Arc::from([]),
    };
    Arc::from([need(301, "Bait", 2), need(302, "Feather", 1)])
}

fn withdraw_trip(fixture: &BaitFixture) -> Gatherer {
    let mut gatherer = fixture.gatherer(GatherRetained::default());
    gatherer.needs_validate = false;
    gatherer.scratch.needs = Some(withdraw_needs());
    gatherer.trip = TripStep::Supply {
        return_to_work: true,
        deposit_products: false,
    };
    gatherer
}

fn withdraw_clicks(ledger: &Option<Box<crate::native::ledger::Ledger>>) -> usize {
    ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .filter(|action| {
            matches!(
                &action.effect,
                crate::native::HostEffect::Interaction(
                    crate::interact::InteractReq::WithdrawX { .. }
                )
            )
        })
        .count()
}

#[test]
fn gatherer_interrupts_withdraw_at_transfer_boundary() {
    if !api::test_support::game_data_available() {
        return;
    }
    let fixture = BaitFixture::new();
    let mut gatherer = withdraw_trip(&fixture);
    let mut snapshot = fixture.snapshot;
    snapshot.seed_inventory(vec![antipoison_item(0)], 28);
    snapshot.seed_bank_observation(
        10,
        1,
        Some(vec![
            bank_item(301, "Bait", 5, 0),
            bank_item(302, "Feather", 3, 1),
        ]),
        Vec::new(),
    );
    let mut ledger = None;

    with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.cx.condition = Some(clear());
        gatherer.tick(tick).unwrap();
        assert!(matches!(
            tick.cx
                .ledger
                .as_ref()
                .unwrap()
                .outbox
                .last()
                .map(|action| &action.effect),
            Some(crate::native::HostEffect::Interaction(
                crate::interact::InteractReq::SetNoteMode { on: false, .. }
            ))
        ));
        accept_last_interaction(tick);
    });
    with_tick(&snapshot, &mut ledger, 2, |tick| {
        tick.cx.condition = Some(clear());
        gatherer.tick(tick).unwrap();
        assert!(matches!(
            tick.cx
                .ledger
                .as_ref()
                .unwrap()
                .outbox
                .last()
                .map(|action| &action.effect),
            Some(crate::native::HostEffect::Interaction(
                crate::interact::InteractReq::WithdrawX {
                    bank_item_id: 301,
                    count: 2,
                    ..
                }
            ))
        ));
        accept_last_interaction(tick);
    });
    with_tick(&snapshot, &mut ledger, 3, |tick| {
        tick.cx.condition = Some(clear());
        gatherer.tick(tick).unwrap();
    });
    assert!(gatherer.active_transfer_in_flight());

    with_tick(&snapshot, &mut ledger, 4, |tick| {
        tick.cx.condition = Some(onset());
        tick.cx.poison_unknown_pulse = 11;
        tick.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
        gatherer.tick(tick).unwrap();
    });
    assert!(gatherer.survival_interrupting);
    assert!(gatherer.active_transfer_in_flight());
    assert_eq!(
        withdraw_clicks(&ledger),
        1,
        "the interrupted withdrawal must not click another bank row before settling"
    );

    snapshot.seed_inventory(vec![antipoison_item(0), held(301, "Bait", 2, 1)], 28);
    snapshot.seed_bank_observation(
        10,
        2,
        Some(vec![
            bank_item(301, "Bait", 3, 0),
            bank_item(302, "Feather", 3, 1),
        ]),
        Vec::new(),
    );
    with_tick(&snapshot, &mut ledger, 5, |tick| {
        tick.cx.condition = Some(onset());
        tick.cx.poison_unknown_pulse = 11;
        tick.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
        gatherer.tick(tick).unwrap();
    });
    let outbox = &ledger.as_ref().unwrap().outbox;
    assert!(matches!(
        outbox.last().map(|action| &action.effect),
        Some(crate::native::HostEffect::Interaction(
            crate::interact::InteractReq::Held { action, .. }
        )) if action == "Drink"
    ));
    assert!(!gatherer.survival_interrupting);
}

#[test]
fn need_does_not_block_interrupted_transfer_polling_to_boundary() {
    if !api::test_support::game_data_available() {
        return;
    }
    use crate::native::{NativePhase, ScriptFlow};
    let fixture = BaitFixture::new();
    let mut gatherer = withdraw_trip(&fixture);
    let mut snapshot = fixture.snapshot;
    snapshot.seed_bank_observation(
        10,
        1,
        Some(vec![
            bank_item(301, "Bait", 5, 0),
            bank_item(302, "Feather", 3, 1),
        ]),
        Vec::new(),
    );
    let mut ledger = None;

    with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.cx.condition = Some(clear());
        gatherer.tick(tick).unwrap();
        accept_last_interaction(tick);
    });
    with_tick(&snapshot, &mut ledger, 2, |tick| {
        tick.cx.condition = Some(clear());
        gatherer.tick(tick).unwrap();
        assert!(matches!(
            tick.cx
                .ledger
                .as_ref()
                .unwrap()
                .outbox
                .last()
                .map(|action| &action.effect),
            Some(crate::native::HostEffect::Interaction(
                crate::interact::InteractReq::WithdrawX {
                    bank_item_id: 301,
                    count: 2,
                    ..
                }
            ))
        ));
        accept_last_interaction(tick);
    });
    with_tick(&snapshot, &mut ledger, 3, |tick| {
        tick.cx.condition = Some(clear());
        gatherer.tick(tick).unwrap();
    });
    assert!(gatherer.active_transfer_in_flight());

    with_tick(&snapshot, &mut ledger, 4, |tick| {
        tick.cx.condition = Some(onset());
        tick.cx.poison_unknown_pulse = 11;
        tick.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
        assert!(matches!(gatherer.tick(tick).unwrap(), ScriptFlow::Continue));
    });
    assert!(gatherer.active_transfer_in_flight());
    assert!(gatherer.survival_interrupting);
    assert_eq!(
        gatherer.survival.report().decision,
        crate::survival::Decision::Needed
    );

    let mut inventory = snapshot.inventory().to_vec();
    inventory.push(held(301, "Bait", 2, 1));
    snapshot.seed_inventory(inventory, 28);
    snapshot.seed_bank_observation(
        10,
        2,
        Some(vec![
            bank_item(301, "Bait", 3, 0),
            bank_item(302, "Feather", 3, 1),
        ]),
        Vec::new(),
    );
    with_tick(&snapshot, &mut ledger, 5, |tick| {
        tick.cx.condition = Some(onset());
        tick.cx.poison_unknown_pulse = 11;
        tick.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
        assert!(matches!(gatherer.tick(tick).unwrap(), ScriptFlow::Continue));
    });

    assert!(!gatherer.active_transfer_in_flight());
    assert!(!gatherer.survival_interrupting);
    assert_eq!(gatherer.phase(), ("parked", NativePhase::Waiting));
    assert_eq!(
        gatherer.survival.report().decision,
        crate::survival::Decision::Parked
    );
    let outbox = &ledger.as_ref().unwrap().outbox;
    assert!(
        !outbox.iter().any(|action| matches!(
            &action.effect,
            crate::native::HostEffect::Interaction(crate::interact::InteractReq::WithdrawX {
                bank_item_id: 302,
                ..
            })
        )),
        "the boundary must not begin the second withdrawal"
    );
}

#[test]
fn gatherer_interrupts_deposit_at_transfer_boundary() {
    if !api::test_support::game_data_available() {
        return;
    }
    let fixture = BaitFixture::new();
    let mut gatherer = fixture.gatherer(GatherRetained::default());
    gatherer.needs_validate = false;
    gatherer.trip = TripStep::Deposit;
    let mut snapshot = fixture.snapshot;
    let ids = [201, 202, 203];
    let mut inventory: Vec<_> = ids
        .iter()
        .enumerate()
        .map(|(slot, id)| held(*id, "Raw fish", 1, i32::try_from(slot).unwrap()))
        .collect();
    inventory.push(antipoison_item(3));
    let bank_side: Vec<_> = inventory
        .iter()
        .cloned()
        .map(|mut item| {
            item.action_family = ItemActionFamily::Component;
            item.actions = vec![Some("Deposit-All".into())];
            item
        })
        .collect();
    snapshot.seed_inventory(inventory, 28);
    snapshot.seed_bank_observation(10, 1, Some(Vec::new()), bank_side);
    let mut ledger = None;
    let product_clicks = |ledger: &Option<Box<crate::native::ledger::Ledger>>| {
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .filter(|action| {
                matches!(
                    &action.effect,
                    crate::native::HostEffect::Interaction(
                        crate::interact::InteractReq::InvButton { id: 201..=203, .. }
                    )
                )
            })
            .count()
    };

    with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.cx.condition = Some(clear());
        tick.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
        gatherer.tick(tick).unwrap();
        assert!(matches!(
            tick.cx
                .ledger
                .as_ref()
                .unwrap()
                .outbox
                .last()
                .map(|action| &action.effect),
            Some(crate::native::HostEffect::Interaction(
                crate::interact::InteractReq::InvButton { id: 201, .. }
            ))
        ));
        accept_last_interaction(tick);
    });
    let outbox_len = ledger.as_ref().unwrap().outbox.len();
    with_tick(&snapshot, &mut ledger, 2, |tick| {
        tick.cx.condition = Some(clear());
        tick.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
        gatherer.tick(tick).unwrap();
        assert_eq!(
            tick.cx.ledger.as_ref().unwrap().outbox.len(),
            outbox_len,
            "the in-flight deposit must not send another click"
        );
    });
    assert!(gatherer.active_transfer_in_flight());
    with_tick(&snapshot, &mut ledger, 3, |tick| {
        tick.cx.condition = Some(clear());
        tick.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
        gatherer.tick(tick).unwrap();
    });
    assert!(gatherer.active_transfer_in_flight());

    with_tick(&snapshot, &mut ledger, 4, |tick| {
        tick.cx.condition = Some(onset());
        tick.cx.poison_unknown_pulse = 11;
        tick.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
        gatherer.tick(tick).unwrap();
    });
    assert!(gatherer.survival_interrupting);
    assert!(gatherer.active_transfer_in_flight());
    // The drink waits for the transfer's boundary, so the status row shows
    // the need, not a drink in progress (design A6.6; Argon review finding 13).
    assert_eq!(
        gatherer.survival.report().decision,
        crate::survival::Decision::Needed
    );
    assert_eq!(
        product_clicks(&ledger),
        1,
        "the interrupted deposit must not click another product before settling"
    );

    let product = |id, count, container, slot| ItemView {
        def: def(id, "Raw fish"),
        container,
        action_family: if container == ItemContainer::Bank {
            ItemActionFamily::Component
        } else {
            ItemActionFamily::Held
        },
        slot,
        count,
        actions: Vec::new(),
        component_id: 3214,
    };
    snapshot.seed_inventory(
        vec![
            product(201, 0, ItemContainer::Inventory, 0),
            product(202, 1, ItemContainer::Inventory, 1),
            product(203, 1, ItemContainer::Inventory, 2),
            antipoison_item(3),
        ],
        28,
    );
    snapshot.seed_bank_observation(
        10,
        2,
        Some(vec![product(201, 1, ItemContainer::Bank, 0)]),
        Vec::new(),
    );
    with_tick(&snapshot, &mut ledger, 5, |tick| {
        tick.cx.condition = Some(onset());
        tick.cx.poison_unknown_pulse = 11;
        tick.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
        gatherer.tick(tick).unwrap();
    });
    let outbox = &ledger.as_ref().unwrap().outbox;
    assert!(matches!(
        outbox.last().map(|action| &action.effect),
        Some(crate::native::HostEffect::Interaction(
            crate::interact::InteractReq::Held { action, .. }
        )) if action == "Drink"
    ));
    assert!(!gatherer.survival_interrupting);
}

const RELOG_BANK_TILE: WorldTile = WorldTile {
    x: 3245,
    z: 3423,
    level: 0,
};

/// The Draynor booth the relog tests stand at, as the scene lists it.
fn relog_booth_locs() -> Vec<api::snapshot::LocView> {
    vec![api::snapshot::LocView {
        typecode: 2213,
        info: 0,
        id: 2213,
        name: Some("Bank booth".to_owned()),
        description: None,
        actions: vec![Some("Use-quickly".to_owned())],
        tile: RELOG_BANK_TILE,
        distance: 0,
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
    }]
}

/// The bait fixture with a three-bank roster, Draynor first, so a pick has
/// banks to choose from (the plain fixture has an empty roster).
fn roster_fixture() -> BaitFixture {
    let tile = |x, z| WorldTile { x, z, level: 0 };
    BaitFixture::with_banks(Arc::new(api::named_banks::NamedBankFacts::from_banks(
        vec![
            api::named_banks::NamedBank::new("Draynor", RELOG_BANK_TILE),
            api::named_banks::NamedBank::new("Varrock East", tile(3253, 3420)),
            api::named_banks::NamedBank::new("Lumbridge", tile(3208, 3220)),
        ],
    )))
}

/// A bank trip driven by the Gatherer's own ticks. The host side answers what
/// the Gatherer queues the way the engine would: the bank pick selects the
/// Draynor stand, a walk arrives, the open delivers the bank's table (a new
/// bank session each time), the withdrawal and deposit clicks move items, the
/// close empties the modal.
struct Trip {
    gatherer: Gatherer,
    snapshot: api::snapshot::GameSnapshot,
    ledger: Option<Box<crate::native::ledger::Ledger>>,
    tick: u64,
    bank_session: u64,
    held_bait: i32,
    deposited: i32,
    picks: u32,
    open_clicks: Vec<u32>,
    bait: (i32, Arc<str>),
    product: i32,
    /// Clicks the engine has not applied yet: they land on the next tick.
    deferred: Vec<crate::interact::InteractReq>,
    /// The open leaves the modal without its item table.
    table_withheld: bool,
    /// The host never sends the open click, so no modal ever shows.
    never_open: bool,
}

impl Trip {
    fn new(fixture: BaitFixture, trip: TripStep) -> Self {
        let bait = fixture.prepared.supply.bait.clone().unwrap();
        let product = fixture.prepared.products[0];
        let mut gatherer = fixture.gatherer(GatherRetained::default());
        gatherer.needs_validate = false;
        gatherer.tool = ToolState {
            id: 307,
            worn: false,
        };
        gatherer.trip = trip;
        let mut snapshot = fixture.snapshot;
        snapshot.seed_locs(relog_booth_locs());
        if trip == TripStep::Deposit {
            snapshot.seed_inventory(
                vec![
                    held(307, "Fishing rod", 1, 0),
                    held(product, "Raw fish", 1, 2),
                ],
                28,
            );
        }
        Self {
            gatherer,
            snapshot,
            ledger: None,
            tick: 0,
            bank_session: 0,
            held_bait: 0,
            deposited: 0,
            picks: 0,
            open_clicks: vec![0],
            bait: (bait.id, Arc::clone(&bait.name)),
            product,
            deferred: Vec::new(),
            table_withheld: false,
            never_open: false,
        }
    }

    fn stage(&self) -> &'static str {
        match &self.gatherer.active {
            Active::Acquire(run) => run.progress().2,
            _ => "-",
        }
    }

    fn pack(&self) -> Vec<ItemView> {
        let mut pack = vec![held(307, "Fishing rod", 1, 0)];
        if self.held_bait > 0 {
            pack.push(held(self.bait.0, &self.bait.1, self.held_bait, 1));
        }
        if self.deposited == 0 && self.gatherer.trip == TripStep::Deposit {
            pack.push(held(self.product, "Raw fish", 1, 2));
        }
        pack
    }

    /// The bank's side view of the pack, with its deposit options.
    fn side(&self) -> Vec<ItemView> {
        self.pack()
            .into_iter()
            .map(|mut item| {
                item.action_family = ItemActionFamily::Component;
                item.actions = vec![Some("Deposit-All".into())];
                item
            })
            .collect()
    }

    fn open_page(&mut self) {
        let (table, side) = (self.table(), self.side());
        self.snapshot
            .seed_bank_observation(10, self.bank_session, Some(table), side);
    }

    fn apply(&mut self, request: crate::interact::InteractReq) {
        match request {
            crate::interact::InteractReq::WithdrawX { count, .. } => {
                self.held_bait += count;
                let pack = self.pack();
                self.snapshot.seed_inventory(pack, 28);
                self.open_page();
            }
            crate::interact::InteractReq::InvButton { .. } => {
                self.deposited += 1;
                let pack = self.pack();
                self.snapshot.seed_inventory(pack, 28);
                self.open_page();
            }
            _ => {}
        }
    }

    fn table(&self) -> Vec<ItemView> {
        vec![bank_item(self.bait.0, &self.bait.1, 20 - self.held_bait, 0)]
    }

    /// One Gatherer tick, then the host's answers to what it queued.
    fn step(&mut self) {
        self.tick_and_answer(true);
    }

    fn tick_and_answer(&mut self, expect_running: bool) {
        use crate::native::HostEffect;
        for request in std::mem::take(&mut self.deferred) {
            self.apply(request);
        }
        self.tick += 1;
        let tick = self.tick;
        with_tick(&self.snapshot, &mut self.ledger, tick, |native| {
            self.gatherer.tick(native).unwrap()
        });
        assert!(
            !expect_running || self.gatherer.failure.is_none(),
            "{:?} at {}",
            self.gatherer.failure,
            self.gatherer.last_event
        );
        let Some(mut owned) = self.ledger.take() else {
            return;
        };
        let ledger = &mut owned;
        let stamp = |run, tick| api::quest_progress::EvidenceStamp {
            run,
            tick,
            sequence: tick,
        };
        let mut arrived_at = None;
        let mut opened = false;
        let mut closed = false;
        let mut unanswered = Vec::new();
        let mut pending = Vec::new();
        while let Some(action) = ledger.outbox.pop() {
            pending.push(action);
        }
        pending.reverse();
        for action in pending {
            match &action.effect {
                HostEffect::BankPick(_) => {
                    self.picks += 1;
                    let access = Arc::new(crate::bank::BankStandAccess {
                        bank: api::named_banks::NamedBank::new("Draynor", RELOG_BANK_TILE),
                        stand_tile: RELOG_BANK_TILE,
                        kind: crate::bank::AccessKind::Booth,
                        stand_op: 2,
                        name: Some(Arc::from("Bank booth")),
                        choose: None,
                    });
                    ledger.complete_bank_pick(
                        &action.authority(),
                        crate::native_bank::BankPickReceipt {
                            request_id: action.request_id.get(),
                            evidence: stamp(action.run(), tick),
                            outcome: crate::native_bank::BankPickOutcome::Selected(
                                crate::native_bank::SelectedBank {
                                    bank_index: 0,
                                    access_tile: RELOG_BANK_TILE,
                                    kind: crate::native_bank::PickKind::Reachable,
                                    access: Some(access),
                                },
                            ),
                        },
                    );
                }
                HostEffect::Walk(request) => {
                    let goal = request
                        .candidate_goals
                        .as_ref()
                        .map_or(request.target, |goals| goals.as_slice()[0]);
                    if request.target != RELOG_BANK_TILE {
                        // Only the return walk moves the character.
                        arrived_at = Some(goal);
                    }
                    ledger.walk = Some(WalkReceipt {
                        goal: request.candidate_goals.is_some().then_some(goal),
                        request_id: action.request_id.get(),
                        evidence: stamp(action.run(), tick),
                        end: WalkEnd::Arrived,
                        blocked: None,
                        detail: None,
                        refusal: None,
                        assessment: None,
                        escape: None,
                        prereq_allow: None,
                    });
                }
                HostEffect::Interaction(crate::interact::InteractReq::OpenStand { .. })
                    if self.never_open =>
                {
                    // The host never sends the click: it stays queued.
                    unanswered.push(action);
                }
                HostEffect::Interaction(request) => {
                    ledger.complete_interaction(
                        &action.authority(),
                        crate::native::InteractionReceipt {
                            request_id: action.request_id.get(),
                            evidence: stamp(action.run(), tick),
                            accepted: true,
                            chat_since: 0,
                        },
                    );
                    match request {
                        crate::interact::InteractReq::OpenStand { .. } => {
                            *self.open_clicks.last_mut().unwrap() += 1;
                            self.bank_session += 1;
                            opened = true;
                        }
                        crate::interact::InteractReq::WithdrawX { .. }
                        | crate::interact::InteractReq::InvButton { .. } => {
                            self.deferred.push(request.clone());
                        }
                        crate::interact::InteractReq::CloseModal
                        | crate::interact::InteractReq::Close => {
                            closed = true;
                        }
                        _ => {}
                    }
                }
                _ => panic!("unexpected host effect in the trip"),
            }
        }
        owned.outbox.extend(unanswered);
        self.ledger = Some(owned);
        if opened && self.table_withheld {
            self.snapshot
                .seed_bank_observation(10, self.bank_session, None, Vec::new());
        } else if opened && !self.never_open {
            self.open_page();
        }
        if closed {
            self.snapshot
                .seed_bank_observation(-1, self.bank_session, None, Vec::new());
            self.snapshot.seed_side_modal(-1);
        }
        if let Some(goal) = arrived_at {
            self.snapshot
                .seed_local_player(crate::quester::families::tests::local_player(goal));
        }
    }

    fn run_until(&mut self, what: &str, done: impl Fn(&Trip) -> bool) {
        for _ in 0..80 {
            if done(self) {
                return;
            }
            self.step();
        }
        panic!(
            "never reached {what}: trip {:?}, stage {}, event {}",
            self.gatherer.trip,
            self.stage(),
            self.gatherer.last_event
        );
    }

    /// The session ends: the Gatherer still holds the old run key, and every
    /// later tick carries the new session.
    fn relog(&mut self, modal_without_table: bool) {
        // A click the engine had not applied yet never lands.
        self.deferred.clear();
        if modal_without_table {
            // The relog kept the client's modal fields and dropped the table.
            self.snapshot
                .seed_bank_observation(10, self.bank_session, None, Vec::new());
        } else {
            self.snapshot
                .seed_bank_observation(-1, self.bank_session, None, Vec::new());
            self.snapshot.seed_side_modal(-1);
        }
        self.gatherer.run.session = 0;
        self.open_clicks.push(0);
    }
}

/// A relog inside the Access, Open, Deposit or Withdraw step of a bank trip
/// is information: the trip restarts with a fresh pick and a fresh access
/// walk from the observed position, the old session's attempts are dropped,
/// and the trip goes on to the withdrawal with a plan made from the new
/// session's bank. 0.2.8 kept the cursor and the old plan, and an Open that
/// adopted a modal without a table waited on it again.
fn relog_restarts_the_trip_and_it_finishes(stage: &'static str, modal_without_table: bool) {
    let first = if stage == "Deposit" {
        TripStep::Deposit
    } else {
        TripStep::Supply {
            return_to_work: true,
            deposit_products: false,
        }
    };
    let mut trip = Trip::new(roster_fixture(), first);
    trip.run_until(stage, |trip| trip.stage() == stage);
    assert_eq!(trip.picks, 1, "the first session picked once");
    let old_open_clicks = trip.open_clicks[0];
    trip.relog(modal_without_table);

    trip.step();
    assert_eq!(
        trip.gatherer.scratch.session_restarts, 1,
        "{stage} restarts the bank trip"
    );
    assert_eq!(trip.picks, 2, "{stage} restarts with a fresh bank pick");
    assert!(
        matches!(
            trip.gatherer.trip,
            TripStep::Deposit | TripStep::Supply { .. }
        ),
        "{:?}",
        trip.gatherer.trip
    );
    trip.run_until("the trip's end", |trip| {
        trip.gatherer.trips == 1 && trip.gatherer.trip == TripStep::Idle
    });
    assert!(
        trip.gatherer.failure.is_none(),
        "{:?}",
        trip.gatherer.failure
    );
    assert_eq!(trip.gatherer.trips, 1, "the trip completed once");
    assert_eq!(
        trip.open_clicks[1], 1,
        "the new session's open accesses the stand for real"
    );
    assert!(old_open_clicks <= 1);
    assert!(
        trip.held_bait > 0,
        "the withdrawal is planned again from the new session's bank"
    );
}

#[test]
fn relog_during_access_restarts_the_trip_and_it_finishes() {
    if !api::test_support::game_data_available() {
        return;
    }
    relog_restarts_the_trip_and_it_finishes("Access", false);
}

#[test]
fn relog_during_open_restarts_the_trip_and_it_finishes() {
    if !api::test_support::game_data_available() {
        return;
    }
    relog_restarts_the_trip_and_it_finishes("Open", true);
}

#[test]
fn relog_during_deposit_restarts_the_trip_and_it_finishes() {
    if !api::test_support::game_data_available() {
        return;
    }
    relog_restarts_the_trip_and_it_finishes("Deposit", false);
}

#[test]
fn relog_during_withdraw_restarts_the_trip_and_it_finishes() {
    if !api::test_support::game_data_available() {
        return;
    }
    relog_restarts_the_trip_and_it_finishes("Withdraw", false);
}

fn reason_recorded(trip: &Trip) -> bool {
    matches!(&trip.gatherer.active, Active::Acquire(run) if run.last_access_reason().is_some())
}

/// Each session change inside the trip restarts it, within the budget. A
/// change after the budget is spent ends the run, and the failure keeps the
/// last reason the trip recorded.
#[test]
fn spent_session_restart_budget_ends_in_a_terminal_failure_with_the_last_reason() {
    if !api::test_support::game_data_available() {
        return;
    }
    use crate::native::NativePhase;
    let mut trip = Trip::new(
        roster_fixture(),
        TripStep::Supply {
            return_to_work: true,
            deposit_products: false,
        },
    );
    trip.table_withheld = true;
    for restart in 1..=MAX_TRIP_SESSION_RESTARTS {
        // This session's open never settles: its stand is accessed and the
        // table does not arrive, so the open reports its reason.
        trip.run_until("an open that reports its reason", reason_recorded);
        assert_eq!(trip.gatherer.scratch.session_restarts, restart - 1);
        // The next session change arrives mid-Open.
        trip.relog(true);
        trip.step();
        assert_eq!(trip.gatherer.scratch.session_restarts, restart);
    }
    trip.run_until("an open that reports its reason", reason_recorded);
    trip.relog(true);
    trip.tick += 1;
    let tick = trip.tick;
    with_tick(&trip.snapshot, &mut trip.ledger, tick, |native| {
        trip.gatherer.tick(native).unwrap()
    });
    let failure = trip
        .gatherer
        .failure
        .as_ref()
        .expect("the budget is spent, so the trip fails");
    assert_eq!(failure.code.as_ref(), "bank-unavailable");
    assert!(
        failure.message.contains("bank item table unavailable"),
        "the last reason is kept: {}",
        failure.message
    );
    assert_eq!(trip.gatherer.phase(), ("blocked", NativePhase::Blocked));
}

/// The bank never opens: the open is retried on observed ticks, then the trip
/// fails with the last reason the retried step recorded.
#[test]
fn gatherer_caps_timed_out_bank_open_attempts() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut trip = Trip::new(
        roster_fixture(),
        TripStep::Supply {
            return_to_work: true,
            deposit_products: false,
        },
    );
    trip.never_open = true;
    trip.snapshot.seed_locs(Vec::new());
    for _ in 0..80 {
        if trip.gatherer.failure.is_some() {
            break;
        }
        trip.tick_and_answer(false);
    }
    let failure = trip
        .gatherer
        .failure
        .as_ref()
        .expect("open retries are capped");
    assert_eq!(failure.code.as_ref(), "bank-unavailable");
    assert_eq!(
        failure.message.as_ref(),
        "bank-unavailable: bank did not open after 3 attempts; last reason: bank did not open"
    );
}

/// A survival need cancels the open being retried and drinks, instead of
/// waiting out the open's bound.
#[test]
fn gatherer_cancels_open_retry_for_drink() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut trip = Trip::new(
        roster_fixture(),
        TripStep::Supply {
            return_to_work: true,
            deposit_products: false,
        },
    );
    trip.never_open = true;
    trip.run_until("the open", |trip| trip.stage() == "Open");
    // The open is under way, with its click still unsent.
    trip.step();
    assert_eq!(trip.stage(), "Open");
    trip.snapshot
        .seed_inventory(vec![held(307, "Fishing rod", 1, 0), antipoison_item(1)], 28);
    trip.snapshot.seed_tick(20);
    trip.tick += 1;
    let tick = trip.tick;
    with_tick(&trip.snapshot, &mut trip.ledger, tick, |native| {
        native.cx.condition = Some(onset());
        native.cx.poison_unknown_pulse = 11;
        native.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
        trip.gatherer.tick(native).unwrap();
    });
    assert_eq!(
        trip.gatherer.survival.report().decision,
        crate::survival::Decision::Drinking
    );
    assert!(matches!(trip.gatherer.active, Active::Tend(_)));
    assert!(matches!(
        trip.ledger.as_ref().unwrap().outbox.last().map(|action| &action.effect),
        Some(crate::native::HostEffect::Interaction(
            crate::interact::InteractReq::Held { action, .. }
        )) if action == "Drink"
    ));
}
