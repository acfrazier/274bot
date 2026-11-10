use super::*;
use crate::bank::{AccessKind, BankPickReceipt, BankStandAccess, PickKind, SelectedBank};
use crate::native::ledger::{HostEffect, Ledger};
use crate::native::{ActionError, InteractionReceipt, WalkEnd};
use crate::native_bank::{BankAction, BankItem};
use api::bank_memory::{BankMemory, Origin};
use api::named_banks::{BankPreferences, NamedBank};
use api::quest_progress::EvidenceStamp;
use api::selected::RunKey;
use api::snapshot::{
    GameSnapshot, ItemActionFamily, ItemContainer, ItemView, LocLayer, LocView, WorldTile,
};
use std::sync::Arc;
use std::task::Poll;

const ITEM: i32 = 42;
const PRODUCT: i32 = 43;
const OTHER: i32 = 44;
const BANK_COMPONENT: i32 = 10;
const STAND: WorldTile = WorldTile {
    x: 3210,
    z: 3210,
    level: 0,
};

type TestLedger = Option<Box<Ledger>>;

fn stamp(tick: u64) -> EvidenceStamp {
    EvidenceStamp {
        run: RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        tick,
        sequence: tick,
    }
}

fn item(id: i32, name: &str, count: i32, container: ItemContainer, actions: &[&str]) -> ItemView {
    let component = matches!(container, ItemContainer::Bank | ItemContainer::BankSide);
    ItemView {
        def: crate::quester::families::tests::def(id, name),
        container,
        action_family: if component {
            ItemActionFamily::Component
        } else {
            ItemActionFamily::Held
        },
        slot: 0,
        count,
        actions: actions
            .iter()
            .map(|action| Some((*action).to_owned()))
            .collect(),
        component_id: if component { 7 } else { 3214 },
    }
}

fn bank_row(id: i32, name: &str, count: i32) -> ItemView {
    item(
        id,
        name,
        count,
        ItemContainer::Bank,
        &["Withdraw-X", "Withdraw-1"],
    )
}

fn side_row(id: i32, name: &str, count: i32) -> ItemView {
    item(
        id,
        name,
        count,
        ItemContainer::BankSide,
        &["Deposit-1", "Deposit-5", "Deposit-10", "Deposit-All"],
    )
}

fn fixture() -> (
    GameSnapshot,
    NamedBank,
    Arc<api::named_banks::NamedBankFacts>,
) {
    let bank = NamedBank::new("Test bank", STAND);
    let banks = Arc::new(api::named_banks::NamedBankFacts::from_banks(vec![bank]));
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_local_player(crate::quester::families::tests::local_player(STAND));
    snapshot.seed_locs(vec![LocView {
        id: 2213,
        name: Some("Bank booth".into()),
        actions: vec![Some("Use-quickly".into())],
        tile: STAND,
        distance: 0,
        typecode: 0,
        info: 0,
        description: None,
        layer: LocLayer::GroundDecoration,
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
    snapshot.seed_inventory(Vec::new(), 28);
    snapshot.seed_equipment(Vec::new());
    (snapshot, bank, banks)
}

fn open_bank(snapshot: &mut GameSnapshot, rows: Option<Vec<ItemView>>, side: Vec<ItemView>) {
    snapshot.seed_bank_observation(BANK_COMPONENT, 1, rows, side);
}

fn request(
    stage: BankStage,
    needs: Vec<Need>,
    deposit: Option<DepositSpec>,
    bank: Option<NamedBank>,
    required: bool,
    partial_ok: bool,
    close: CloseBank,
) -> AcquireRequest {
    AcquireRequest {
        needs: needs.into(),
        deposit,
        bank: BankChoice {
            bank,
            explicit: None,
            required,
            preferences: BankPreferences::default(),
        },
        stage,
        partial_ok,
        close,
        vendor: VendorChoice::None,
        coins: CoinPolicy::None,
        walk: WalkOptions::default(),
        required_after: stamp(0),
    }
}

fn authored_request(
    bank: Option<NamedBank>,
    required: bool,
    actions: Vec<BankAction>,
) -> AcquireRequest {
    request(
        BankStage::Authored(actions.into()),
        Vec::new(),
        None,
        bank,
        required,
        true,
        CloseBank::LeaveOpen,
    )
}

fn need(
    id: i32,
    name: &str,
    target: i32,
    minimum: i32,
    presence: Presence,
    kind: NeedKind,
    sources: SourceSet,
) -> Need {
    Need {
        item: id,
        name: Arc::from(name),
        target,
        minimum,
        presence,
        kind,
        sources,
        alternatives: Arc::from([]),
    }
}

fn new_run(
    request: AcquireRequest,
    snapshot: &GameSnapshot,
    memory: Option<&BankMemory>,
    ledger: &mut TestLedger,
    banks: &Arc<api::named_banks::NamedBankFacts>,
) -> AcquireRun {
    crate::quester::families::tests::with_tick_bank(snapshot, memory, ledger, 1, |tick| {
        let plan = match plan::plan(&request, &tick.cx.snapshot().stock(), WorldFacts::default())
            .expect("fixture acquisition request plans")
        {
            AcquireVerdict::Plan(plan) => plan,
            AcquireVerdict::Satisfied => panic!("fixture request must produce a run plan"),
            AcquireVerdict::Blocked(block) => {
                panic!("fixture request unexpectedly blocked: {}", block.message)
            }
        };
        AcquireRun::new(request, plan, banks, &tick.cx)
    })
}

fn poll_at(
    run: &mut AcquireRun,
    snapshot: &GameSnapshot,
    memory: Option<&BankMemory>,
    ledger: &mut TestLedger,
    tick_number: u64,
    gate: OpenGate,
) -> Poll<Result<AcquireOutcome, ActionError>> {
    crate::quester::families::tests::with_tick_scene_bank(
        snapshot,
        memory,
        ledger,
        tick_number,
        |tick| run.poll(tick, gate),
    )
}

fn first_effect(ledger: &TestLedger) -> &HostEffect {
    &ledger
        .as_ref()
        .expect("native action created ledger")
        .outbox[0]
        .effect
}

fn complete_pick(ledger: &mut TestLedger, bank: NamedBank, tick: u64) {
    let ledger = ledger.as_mut().expect("bank selection queued a pick");
    let action = ledger.outbox.remove(0);
    assert!(matches!(&action.effect, HostEffect::BankPick(_)));
    let authority = action.authority();
    ledger.complete_bank_pick(
        &authority,
        BankPickReceipt {
            request_id: authority.request_id().get(),
            evidence: EvidenceStamp {
                run: authority.run(),
                tick,
                sequence: tick,
            },
            outcome: crate::native_bank::BankPickOutcome::Selected(SelectedBank {
                bank_index: 0,
                access_tile: STAND,
                kind: PickKind::Reachable,
                access: Some(Arc::new(BankStandAccess {
                    bank,
                    stand_tile: STAND,
                    kind: AccessKind::Booth,
                    stand_op: 1,
                    name: None,
                    choose: None,
                })),
            }),
        },
    );
}

fn complete_interaction(ledger: &mut TestLedger, tick: u64) -> HostEffect {
    let ledger = ledger.as_mut().expect("native interaction created ledger");
    let action = ledger.outbox.remove(0);
    assert!(matches!(&action.effect, HostEffect::Interaction(_)));
    let authority = action.authority();
    ledger.complete_interaction(
        &authority,
        InteractionReceipt {
            request_id: authority.request_id().get(),
            evidence: EvidenceStamp {
                run: authority.run(),
                tick,
                sequence: tick,
            },
            accepted: true,
            chat_since: 0,
        },
    );
    action.effect
}

fn done(result: Poll<Result<AcquireOutcome, ActionError>>) -> AcquireReceipt {
    match result {
        Poll::Ready(Ok(AcquireOutcome::Done(receipt))) => receipt,
        _ => panic!("acquisition did not finish successfully"),
    }
}

fn is_open_stand(effect: &HostEffect) -> bool {
    matches!(
        effect,
        HostEffect::Interaction(crate::interact::InteractReq::OpenStand { .. })
    )
}

fn is_close(effect: &HostEffect) -> bool {
    matches!(
        effect,
        HostEffect::Interaction(crate::interact::InteractReq::Close)
    )
}

#[test]
fn authored_empty_actions_open_required_bank_and_leave_it_open() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, bank, banks) = fixture();
    let mut ledger = None;
    let mut run = new_run(
        authored_request(Some(bank), true, Vec::new()),
        &snapshot,
        None,
        &mut ledger,
        &banks,
    );

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    assert!(matches!(first_effect(&ledger), HostEffect::BankPick(_)));
    complete_pick(&mut ledger, bank, 1);

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 2, OpenGate::Open).is_pending());
    assert!(is_open_stand(first_effect(&ledger)));
    assert!(is_open_stand(&complete_interaction(&mut ledger, 2)));
    open_bank(&mut snapshot, Some(Vec::new()), Vec::new());

    let receipt = done(poll_at(
        &mut run,
        &snapshot,
        None,
        &mut ledger,
        3,
        OpenGate::Open,
    ));
    assert!(receipt.complete);
    assert!(receipt.bank_open);
    assert!(receipt.last_bank.is_none());
    assert!(ledger
        .as_ref()
        .is_none_or(|ledger| ledger.outbox.is_empty()));
}
struct CallerNoStore;

impl api::local_state::LocalStatePort for CallerNoStore {
    fn lease(
        &self,
        identity: &api::local_state::DomainIdentity,
    ) -> Result<api::local_state::DomainLease, api::local_state::ErrorCode> {
        Err(api::local_state::ErrorCode::NotReady {
            domain: identity.domain(),
        })
    }

    fn submit(&self, edit: api::local_state::DomainEdit) -> api::local_state::SubmitReceipt {
        api::local_state::SubmitReceipt::Refused(api::local_state::ErrorCode::Busy {
            domain: edit.identity().domain(),
        })
    }

    fn observe(&self, _: api::identity::RequestId) -> Option<api::local_state::WriteSettlement> {
        None
    }
}

impl crate::local_state::StatePort for CallerNoStore {
    fn lease_version(
        &self,
        identity: &api::local_state::DomainIdentity,
    ) -> Result<api::local_state::StoreVersion, api::local_state::ErrorCode> {
        api::local_state::LocalStatePort::lease(self, identity).map(|lease| lease.version())
    }
}

static CALLER_STATE: std::sync::LazyLock<crate::local_state::SlotState> =
    std::sync::LazyLock::new(|| {
        crate::local_state::SlotState::new(
            Arc::new(CallerNoStore),
            api::local_state::AccountKey::try_new("local-289", "acquire_test").unwrap(),
            api::local_state::ContentScope::try_new(
                api::selected::ClientRevision::R289,
                "test-content",
            )
            .unwrap(),
        )
    });

fn prepare_kitchen_journal() {
    static PREPARED: std::sync::LazyLock<()> = std::sync::LazyLock::new(|| {
        let _ = api::selected::FamilyPreparation::run(|worker| {
            let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
            data.prepare_quests(worker).unwrap();
            data.prepare_prereq_steps(worker).unwrap()
        })
        .unwrap()
        .join()
        .unwrap();
    });
    let _ = &*PREPARED;
}

fn kitchen_journal_gate() -> api::prereq_steps::ActionableGate {
    use api::prereq_steps::{CrossingId, CrossingKind};
    use api::selected::{FactKey, InclusiveRange, QuestGate, StageWindow};
    api::prereq_steps::ActionableGate {
        gate: QuestGate::Window(StageWindow {
            quest: FactKey::new("blackarmgang"),
            signal: FactKey::new("phoenixgang"),
            values: InclusiveRange {
                min: Some(10),
                max: Some(10),
            },
        }),
        truth: api::selected::Truth::Unknown,
        leg: 0,
        crossing: CrossingId {
            loc_id: 2628,
            option: 1,
            at: WorldTile {
                x: 2788,
                z: 3190,
                level: 0,
            },
            takeoff: WorldTile {
                x: 2788,
                z: 3190,
                level: 0,
            },
            to: WorldTile {
                x: 2788,
                z: 3189,
                level: 0,
            },
            kind: CrossingKind::Door,
        },
    }
}

fn seed_kitchen_journal_page(snapshot: &mut GameSnapshot) {
    snapshot.seed_main_modal(
        crate::quest_journal::ROOT_289,
        vec![
            crate::quest_journal::test_widget(
                crate::quest_journal::TITLE_289,
                "@dre@Shield of Arrav",
            ),
            crate::quest_journal::test_widget(
                8145,
                "@str@I searched the Phoenix Gang Headquarters and found",
            ),
            crate::quest_journal::test_widget(8146, "@str@half of the Shield of Arrav."),
            crate::quest_journal::test_widget(8147, "@red@QUEST COMPLETE!"),
        ],
    );
}

fn poll_with_journal(
    run: &mut AcquireRun,
    snapshot: &GameSnapshot,
    ledger: &mut TestLedger,
    tick_number: u64,
    gate: OpenGate,
) -> Poll<Result<AcquireOutcome, ActionError>> {
    crate::quester::families::tests::with_tick_scene_bank(
        snapshot,
        None,
        ledger,
        tick_number,
        |native| {
            native.state = Some(&CALLER_STATE);
            native.cx.active_now = std::time::Duration::ZERO;
            run.poll(native, gate)
        },
    )
}

fn caller_bank_pick(
    ledger: &TestLedger,
) -> Option<(
    crate::native_bank::BankPickRequest,
    crate::native::ledger::HostAuthority,
)> {
    ledger
        .as_ref()?
        .outbox
        .iter()
        .rev()
        .find_map(|action| match &action.effect {
            HostEffect::BankPick(request) => Some((request.clone(), action.authority())),
            _ => None,
        })
}

fn caller_reissued_bank_pick(
    ledger: &TestLedger,
    original_id: u64,
) -> Option<(
    crate::native_bank::BankPickRequest,
    crate::native::ledger::HostAuthority,
)> {
    ledger.as_ref()?.outbox.iter().rev().find_map(|action| {
        if !action.live() || action.request_id.get() == original_id {
            return None;
        }
        match &action.effect {
            HostEffect::BankPick(request) => Some((request.clone(), action.authority())),
            _ => None,
        }
    })
}

fn publish_caller_bank_pick(
    ledger: &mut TestLedger,
    authority: &crate::native::ledger::HostAuthority,
    tick: u64,
    outcome: crate::native_bank::BankPickOutcome,
) {
    ledger
        .as_mut()
        .expect("bank pick request created ledger")
        .complete_bank_pick(
            authority,
            crate::native_bank::BankPickReceipt {
                request_id: authority.request_id().get(),
                evidence: EvidenceStamp {
                    run: authority.run(),
                    tick,
                    sequence: tick,
                },
                outcome,
            },
        );
}

fn assert_bank_pick_reissued_unchanged(
    original: &crate::native_bank::BankPickRequest,
    reissued: &crate::native_bank::BankPickRequest,
) {
    assert!(Arc::ptr_eq(&original.facts, &reissued.facts));
    assert_eq!(original.facts.as_ref(), reissued.facts.as_ref());
    assert_eq!(original.from, reissued.from);
    assert_eq!(original.preferences, reissued.preferences);
    assert_eq!(original.options, reissued.options);
    assert_eq!(original.eligible, reissued.eligible);
    assert_eq!(original.explicit_bank, reissued.explicit_bank);
    assert_eq!(original.allow_air_fallback, reissued.allow_air_fallback);
}

fn assert_no_acquire_bank_work(ledger: &TestLedger, original_id: u64) {
    for action in &ledger.as_ref().expect("bank pick created ledger").outbox {
        match &action.effect {
            HostEffect::BankPick(_) => {
                assert_eq!(
                    action.request_id.get(),
                    original_id,
                    "no early bank reissue"
                );
            }
            HostEffect::Walk(_) => panic!("bank walk began before selection"),
            HostEffect::Interaction(crate::interact::InteractReq::OpenStand { .. }) => {
                panic!("bank access began before selection")
            }
            _ => {}
        }
    }
}

fn assert_original_allow_acquire(run: &AcquireRun, allow: crate::native::PrereqAllow) {
    match &run.active {
        Active::Select(select) => assert_eq!(select.original_allow(), Some(allow)),
        _ => panic!("acquire run no longer owns the bank selection"),
    }
}

#[test]
fn acquire_reissues_original_bank_pick_after_real_journal_close_and_opens_bank() {
    if !api::test_support::game_data_available() {
        return;
    }
    prepare_kitchen_journal();
    let (mut snapshot, bank, banks) = fixture();
    let start = WorldTile {
        x: STAND.x + 12,
        z: STAND.z,
        level: STAND.level,
    };
    snapshot.seed_local_player(crate::quester::families::tests::local_player(start));
    snapshot.seed_quest_statuses(
        vec![api::snapshot::QuestStatusView {
            name: "Shield of Arrav".into(),
            component_id: 42,
            colour: 0x00f800,
        }],
        true,
    );

    let preferences = api::named_banks::BankPreferences {
        use_mage_bank: true,
        use_zanaris_bank: false,
    };
    let options = crate::native::WalkOptions {
        allow_teleports: crate::native::WalkBit::Forbid,
        allow_wilderness: crate::native::WalkBit::Allow,
        ..crate::native::WalkOptions::default()
    };
    let mut request = authored_request(Some(bank), true, Vec::new());
    request.bank.preferences = preferences;
    request.walk = options;
    let required_after = request.required_after;
    let mut ledger = None;
    let mut run = new_run(request, &snapshot, None, &mut ledger, &banks);
    assert!(poll_with_journal(&mut run, &snapshot, &mut ledger, 1, OpenGate::Open).is_pending());
    let (original_request, original_authority) =
        caller_bank_pick(&ledger).expect("AcquireRun submits its bank pick");
    let original_id = original_authority.request_id().get();
    assert_eq!(original_request.from, start);
    assert_eq!(original_request.preferences, preferences);
    assert_eq!(original_request.options, options);
    assert_eq!(original_request.eligible.as_ref(), &[0]);
    assert_eq!(original_request.explicit_bank, Some(0));
    assert!(!original_request.allow_air_fallback);

    let gate = kitchen_journal_gate();
    let allow = crate::native::PrereqAllow {
        dialogue: true,
        quest_start: false,
    };
    publish_caller_bank_pick(
        &mut ledger,
        &original_authority,
        2,
        crate::native_bank::BankPickOutcome::NeedsStep {
            gates: Arc::from([gate.clone()]),
            allow,
        },
    );
    assert!(poll_with_journal(&mut run, &snapshot, &mut ledger, 2, OpenGate::Open).is_pending());
    assert!(matches!(run.active, Active::Select(_)));
    assert!(run.bank.is_none());
    assert!(run.target.is_none());
    assert_original_allow_acquire(&run, allow);
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        HostEffect::Interaction(crate::interact::InteractReq::IfButton { component_id: 42 })
    ));
    assert_no_acquire_bank_work(&ledger, original_id);

    assert!(matches!(
        complete_interaction(&mut ledger, 2),
        HostEffect::Interaction(crate::interact::InteractReq::IfButton { component_id: 42 })
    ));
    seed_kitchen_journal_page(&mut snapshot);
    assert!(poll_with_journal(&mut run, &snapshot, &mut ledger, 3, OpenGate::Open).is_pending());
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        HostEffect::Interaction(crate::interact::InteractReq::CloseModal)
    ));
    assert!(run.bank.is_none());
    assert_no_acquire_bank_work(&ledger, original_id);

    assert!(matches!(
        complete_interaction(&mut ledger, 3),
        HostEffect::Interaction(crate::interact::InteractReq::CloseModal)
    ));
    snapshot.seed_main_modal(-1, vec![]);
    assert!(poll_with_journal(&mut run, &snapshot, &mut ledger, 4, OpenGate::Open).is_pending());
    assert_eq!(
        CALLER_STATE
            .prereq_snapshot()
            .observe(
                &gate.gate,
                EvidenceStamp {
                    run: original_authority.run(),
                    tick: 3,
                    sequence: 3,
                },
            )
            .map(|fact| fact.truth),
        Some(api::selected::Truth::True),
        "the journal fact is available only after the close is observed"
    );
    assert!(run.bank.is_none());
    assert_no_acquire_bank_work(&ledger, original_id);

    let mut reissued = caller_reissued_bank_pick(&ledger, original_id);
    for tick in 5..=8 {
        if reissued.is_some() {
            break;
        }
        assert!(
            poll_with_journal(&mut run, &snapshot, &mut ledger, tick, OpenGate::Open).is_pending()
        );
        reissued = caller_reissued_bank_pick(&ledger, original_id);
    }
    let (reissued_request, reissued_authority) =
        reissued.expect("a fresh request follows journal proof and observed close");
    assert_ne!(
        reissued_authority.request_id(),
        original_authority.request_id()
    );
    assert_bank_pick_reissued_unchanged(&original_request, &reissued_request);
    assert_eq!(reissued_request.from, start);
    assert!(!run.required_bank_missing);
    assert_original_allow_acquire(&run, allow);
    assert!(!original_authority.live());

    let access = Arc::new(BankStandAccess {
        bank,
        stand_tile: STAND,
        kind: AccessKind::Booth,
        stand_op: 1,
        name: None,
        choose: None,
    });
    publish_caller_bank_pick(
        &mut ledger,
        &reissued_authority,
        9,
        crate::native_bank::BankPickOutcome::Selected(SelectedBank {
            bank_index: 0,
            access_tile: STAND,
            kind: PickKind::Reachable,
            access: Some(access),
        }),
    );
    assert!(poll_with_journal(&mut run, &snapshot, &mut ledger, 10, OpenGate::Open).is_pending());
    assert_eq!(run.bank, Some(bank));
    assert_eq!(run.target, Some(STAND));
    let mut walk_request = ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .find_map(|action| match &action.effect {
            HostEffect::Walk(request) => Some(request.clone()),
            _ => None,
        });
    for tick in 11..=12 {
        if walk_request.is_some() {
            break;
        }
        assert!(
            poll_with_journal(&mut run, &snapshot, &mut ledger, tick, OpenGate::Open).is_pending()
        );
        walk_request =
            ledger
                .as_ref()
                .unwrap()
                .outbox
                .iter()
                .find_map(|action| match &action.effect {
                    HostEffect::Walk(request) => Some(request.clone()),
                    _ => None,
                });
    }
    let walk_request = walk_request.expect("selected bank continues through AcquireRun's walk");
    assert_eq!(walk_request.target, STAND);
    assert_eq!(walk_request.options, options);
    assert_eq!(walk_request.required_after, required_after);

    snapshot.seed_local_player(crate::quester::families::tests::local_player(STAND));
    let walk_request_id = ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .rev()
        .find_map(|action| {
            matches!(&action.effect, HostEffect::Walk(_)).then_some(action.request_id.get())
        })
        .expect("the access walk has a host request");
    let walk_receipt_id =
        crate::quester::families::tests::post_walk_receipt(&mut ledger, 13, WalkEnd::Arrived, None);
    assert_eq!(walk_receipt_id, walk_request_id);
    assert!(poll_with_journal(&mut run, &snapshot, &mut ledger, 13, OpenGate::Open).is_pending());
    let mut opened = ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .any(|action| is_open_stand(&action.effect));
    for tick in 14..=16 {
        if opened {
            break;
        }
        assert!(
            poll_with_journal(&mut run, &snapshot, &mut ledger, tick, OpenGate::Open).is_pending()
        );
        opened = ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .any(|action| is_open_stand(&action.effect));
    }
    assert!(opened, "AcquireRun opens the selected bank after arriving");
    assert!(is_open_stand(&complete_interaction(&mut ledger, 17)));
    open_bank(&mut snapshot, Some(Vec::new()), Vec::new());
    let receipt = done(poll_with_journal(
        &mut run,
        &snapshot,
        &mut ledger,
        18,
        OpenGate::Open,
    ));
    assert!(receipt.complete);
    assert!(receipt.bank_open);
}

#[test]
fn acquire_no_candidate_keeps_unavailable_terminal_outcome() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (snapshot, bank, banks) = fixture();
    let mut ledger = None;
    let mut run = new_run(
        authored_request(Some(bank), true, Vec::new()),
        &snapshot,
        None,
        &mut ledger,
        &banks,
    );
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    let (_, authority) = caller_bank_pick(&ledger).expect("bank pick was emitted");
    publish_caller_bank_pick(
        &mut ledger,
        &authority,
        2,
        crate::native_bank::BankPickOutcome::NoCandidate {
            detail: Arc::from("no routable bank candidate after NoPath"),
        },
    );
    assert!(matches!(
        poll_at(&mut run, &snapshot, None, &mut ledger, 2, OpenGate::Open),
        Poll::Ready(Err(ActionError::Unavailable(message)))
            if message.as_ref() == "no eligible bank"
    ));
}

#[test]
fn quester_bank_access_retries_a_failed_walk_on_the_next_tick() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, bank, banks) = fixture();
    let start = WorldTile {
        x: STAND.x + 12,
        z: STAND.z,
        level: STAND.level,
    };
    snapshot.seed_local_player(crate::quester::families::tests::local_player(start));
    let mut ledger = None;
    let mut run = new_run(
        authored_request(Some(bank), true, Vec::new()),
        &snapshot,
        None,
        &mut ledger,
        &banks,
    )
    .with_quester_walk_retries();

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    assert!(matches!(first_effect(&ledger), HostEffect::BankPick(_)));
    complete_pick(&mut ledger, bank, 1);

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 2, OpenGate::Open).is_pending());
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        HostEffect::Walk(_)
    ));
    let first_request_id = ledger
        .as_ref()
        .unwrap()
        .outbox
        .last()
        .unwrap()
        .request_id
        .get();
    assert_eq!(
        crate::quester::families::tests::post_walk_receipt(
            &mut ledger,
            3,
            WalkEnd::NoPath,
            Some("first bank route failed"),
        ),
        first_request_id
    );
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 3, OpenGate::Open).is_pending());
    let walk_count = |ledger: &TestLedger| {
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .filter(|action| matches!(&action.effect, HostEffect::Walk(_)))
            .count()
    };
    assert_eq!(walk_count(&ledger), 1);

    let moved = WorldTile {
        x: start.x - 1,
        ..start
    };
    snapshot.seed_local_player(crate::quester::families::tests::local_player(moved));
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 4, OpenGate::Open).is_pending());
    assert_ne!(
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .rev()
            .find(|action| matches!(&action.effect, HostEffect::Walk(_)))
            .unwrap()
            .request_id
            .get(),
        first_request_id
    );
    let retry = ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .rev()
        .find(|action| matches!(&action.effect, HostEffect::Walk(_)))
        .unwrap();
    let HostEffect::Walk(request) = &retry.effect else {
        unreachable!();
    };
    assert_eq!(request.target, STAND);
    assert_eq!(request.required_after.tick, 4);

    snapshot.seed_local_player(crate::quester::families::tests::local_player(STAND));
    crate::quester::families::tests::post_walk_receipt(&mut ledger, 5, WalkEnd::Arrived, None);
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 5, OpenGate::Open).is_pending());
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        HostEffect::Interaction(crate::interact::InteractReq::OpenStand { .. })
    ));
}

#[test]
fn hold_blocks_initial_open_and_does_not_advance_active_open() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, bank, banks) = fixture();
    let mut ledger = None;
    let mut run = new_run(
        authored_request(Some(bank), true, Vec::new()),
        &snapshot,
        None,
        &mut ledger,
        &banks,
    );

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Hold).is_pending());
    assert!(matches!(first_effect(&ledger), HostEffect::BankPick(_)));
    complete_pick(&mut ledger, bank, 1);

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 2, OpenGate::Hold).is_pending());
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "Hold must not dispatch OpenStand"
    );
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 3, OpenGate::Open).is_pending());
    assert!(is_open_stand(first_effect(&ledger)));
    complete_interaction(&mut ledger, 3);
    open_bank(&mut snapshot, Some(Vec::new()), Vec::new());

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 4, OpenGate::Hold).is_pending());
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "Hold must leave the active Open unpolled"
    );
    let receipt = done(poll_at(
        &mut run,
        &snapshot,
        None,
        &mut ledger,
        5,
        OpenGate::Open,
    ));
    assert!(receipt.bank_open);
}

#[test]
fn authored_partial_withdrawal_transfers_observed_available_count() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, bank, banks) = fixture();
    open_bank(
        &mut snapshot,
        Some(vec![bank_row(ITEM, "Widget", 2)]),
        Vec::new(),
    );
    let memory = BankMemory::seeded(&[(ITEM, 2)], Origin::Session);
    let action = BankAction::Withdraw {
        item: BankItem {
            id: ITEM,
            name: Arc::from("Widget"),
        },
        qty: 5,
    };
    let strict = authored_request(Some(bank), false, vec![action.clone()]);
    let strict = request(
        strict.stage,
        Vec::new(),
        None,
        Some(bank),
        false,
        false,
        CloseBank::LeaveOpen,
    );
    let mut ledger = None;
    let strict_verdict = crate::quester::families::tests::with_tick_bank(
        &snapshot,
        Some(&memory),
        &mut ledger,
        1,
        |tick| plan::plan(&strict, &tick.cx.snapshot().stock(), WorldFacts::default()).unwrap(),
    );
    assert!(matches!(
        strict_verdict,
        AcquireVerdict::Blocked(AcquireBlock {
            reason: BlockReason::AuthoredShortage,
            ..
        })
    ));

    let partial = request(
        BankStage::Authored(vec![action].into()),
        Vec::new(),
        None,
        Some(bank),
        false,
        true,
        CloseBank::LeaveOpen,
    );
    let mut run = new_run(partial, &snapshot, Some(&memory), &mut ledger, &banks);
    assert!(poll_at(
        &mut run,
        &snapshot,
        Some(&memory),
        &mut ledger,
        1,
        OpenGate::Open
    )
    .is_pending());
    assert!(matches!(
        first_effect(&ledger),
        HostEffect::Interaction(crate::interact::InteractReq::SetNoteMode { on: false, .. })
    ));
    complete_interaction(&mut ledger, 1);
    assert!(poll_at(
        &mut run,
        &snapshot,
        Some(&memory),
        &mut ledger,
        2,
        OpenGate::Open
    )
    .is_pending());
    assert!(matches!(
        first_effect(&ledger),
        HostEffect::Interaction(crate::interact::InteractReq::WithdrawX {
            bank_item_id: ITEM,
            ..
        })
    ));
    assert!(matches!(
        complete_interaction(&mut ledger, 2),
        HostEffect::Interaction(crate::interact::InteractReq::WithdrawX {
            bank_item_id: ITEM,
            ..
        })
    ));

    snapshot.seed_inventory(
        vec![item(ITEM, "Widget", 2, ItemContainer::Inventory, &["Use"])],
        28,
    );
    open_bank(
        &mut snapshot,
        Some(vec![bank_row(ITEM, "Widget", 0)]),
        Vec::new(),
    );
    let receipt = done(poll_at(
        &mut run,
        &snapshot,
        Some(&memory),
        &mut ledger,
        3,
        OpenGate::Open,
    ));
    assert!(
        !receipt.complete,
        "partial fulfillment must remain incomplete"
    );
    assert!(receipt.withdrew_any);
    assert_eq!(
        receipt.last_bank.as_ref().map(|receipt| receipt.complete),
        Some(false)
    );
    assert_eq!(receipt.deposited, 0);
    assert!(receipt.bank_open);
}

#[test]
fn generated_deposit_precedes_live_replan_and_receipts_exact_count() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, bank, banks) = fixture();
    snapshot.seed_inventory(
        vec![item(
            PRODUCT,
            "Product",
            2,
            ItemContainer::Inventory,
            &["Use"],
        )],
        28,
    );
    open_bank(
        &mut snapshot,
        Some(vec![bank_row(ITEM, "Widget", 2)]),
        vec![side_row(PRODUCT, "Product", 2)],
    );
    let need = need(
        ITEM,
        "Widget",
        2,
        2,
        Presence::Held,
        NeedKind::Required,
        SourceSet::PACK | SourceSet::BANK,
    );
    let request = request(
        BankStage::Planned,
        vec![need],
        Some(DepositSpec::Ids {
            products: Arc::from([PRODUCT]),
            keep: Arc::from([]),
        }),
        Some(bank),
        false,
        false,
        CloseBank::LeaveOpen,
    );
    let mut ledger = None;
    let mut run = new_run(request, &snapshot, None, &mut ledger, &banks);

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    assert!(matches!(
        first_effect(&ledger),
        HostEffect::Interaction(crate::interact::InteractReq::InvButton { id: PRODUCT, .. })
    ));
    assert!(matches!(
        complete_interaction(&mut ledger, 1),
        HostEffect::Interaction(crate::interact::InteractReq::InvButton { id: PRODUCT, .. })
    ));
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 2, OpenGate::Open).is_pending());
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "re-plan must wait for observed deposit effects"
    );

    snapshot.seed_inventory(Vec::new(), 28);
    open_bank(
        &mut snapshot,
        Some(vec![
            bank_row(ITEM, "Widget", 2),
            bank_row(PRODUCT, "Product", 2),
        ]),
        Vec::new(),
    );
    let mut withdrawal_tick = None;
    for tick in 3..=6 {
        assert!(poll_at(&mut run, &snapshot, None, &mut ledger, tick, OpenGate::Open).is_pending());
        if ledger
            .as_ref()
            .is_some_and(|ledger| !ledger.outbox.is_empty())
        {
            match first_effect(&ledger) {
                HostEffect::Interaction(crate::interact::InteractReq::SetNoteMode {
                    on: false,
                    ..
                }) => {
                    complete_interaction(&mut ledger, tick);
                }
                HostEffect::Interaction(crate::interact::InteractReq::WithdrawX {
                    bank_item_id: ITEM,
                    ..
                }) => {
                    assert!(matches!(
                        complete_interaction(&mut ledger, tick),
                        HostEffect::Interaction(crate::interact::InteractReq::WithdrawX {
                            bank_item_id: ITEM,
                            ..
                        })
                    ));
                    withdrawal_tick = Some(tick);
                    break;
                }
                _ => panic!("live re-plan dispatched an unexpected effect"),
            }
        }
    }
    let withdrawal_tick =
        withdrawal_tick.expect("live re-plan must withdraw the newly banked need");
    snapshot.seed_inventory(
        vec![item(ITEM, "Widget", 2, ItemContainer::Inventory, &["Use"])],
        28,
    );
    open_bank(
        &mut snapshot,
        Some(vec![bank_row(PRODUCT, "Product", 2)]),
        Vec::new(),
    );
    let receipt = done(poll_at(
        &mut run,
        &snapshot,
        None,
        &mut ledger,
        withdrawal_tick + 1,
        OpenGate::Open,
    ));
    assert!(receipt.complete);
    assert!(receipt.withdrew_any);
    assert_eq!(receipt.deposited, 2);
    assert!(receipt.bank_open);
}

#[test]
fn quester_interrupts_acquire_run_at_transfer_boundary() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, bank, banks) = fixture();
    snapshot.seed_inventory(
        vec![item(
            PRODUCT,
            "Product",
            2,
            ItemContainer::Inventory,
            &["Use"],
        )],
        28,
    );
    open_bank(
        &mut snapshot,
        Some(vec![bank_row(ITEM, "Widget", 2)]),
        vec![side_row(PRODUCT, "Product", 2)],
    );
    let need = need(
        ITEM,
        "Widget",
        2,
        2,
        Presence::Held,
        NeedKind::Required,
        SourceSet::PACK | SourceSet::BANK,
    );
    let request = request(
        BankStage::Planned,
        vec![need],
        Some(DepositSpec::Ids {
            products: Arc::from([PRODUCT]),
            keep: Arc::from([]),
        }),
        Some(bank),
        false,
        false,
        CloseBank::LeaveOpen,
    );
    let mut ledger = None;
    let mut run = new_run(request, &snapshot, None, &mut ledger, &banks);
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    assert!(matches!(
        complete_interaction(&mut ledger, 1),
        HostEffect::Interaction(crate::interact::InteractReq::InvButton { id: PRODUCT, .. })
    ));
    assert!(run.transfer_in_flight());
    crate::quester::families::tests::with_tick_bank(&snapshot, None, &mut ledger, 2, |tick| {
        run.interrupt_transfer(tick.actions);
    });
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 3, OpenGate::Open).is_pending());
    assert!(ledger.as_ref().unwrap().outbox.is_empty());

    snapshot.seed_inventory(Vec::new(), 28);
    open_bank(
        &mut snapshot,
        Some(vec![
            bank_row(ITEM, "Widget", 2),
            bank_row(PRODUCT, "Product", 2),
        ]),
        Vec::new(),
    );
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 4, OpenGate::Open).is_pending());
    assert!(!run.transfer_in_flight());
    assert!(matches!(run.active, Active::None));
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "the boundary poll must not begin a later bank action"
    );

    let (mut snapshot, bank, banks) = fixture();
    open_bank(
        &mut snapshot,
        Some(vec![
            bank_row(ITEM, "Widget", 3),
            bank_row(OTHER, "Other", 1),
        ]),
        Vec::new(),
    );
    let request = authored_request(
        Some(bank),
        false,
        vec![BankAction::WithdrawTo {
            withdrawals: Arc::from([
                crate::native_bank::Withdrawal {
                    id: ITEM,
                    name: Arc::from("Widget"),
                    target: 2,
                },
                crate::native_bank::Withdrawal {
                    id: OTHER,
                    name: Arc::from("Other"),
                    target: 1,
                },
            ]),
        }],
    );
    let mut ledger = None;
    let mut run = new_run(request, &snapshot, None, &mut ledger, &banks);
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    assert!(matches!(
        complete_interaction(&mut ledger, 1),
        HostEffect::Interaction(crate::interact::InteractReq::SetNoteMode { on: false, .. })
    ));
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 2, OpenGate::Open).is_pending());
    assert!(matches!(
        complete_interaction(&mut ledger, 2),
        HostEffect::Interaction(crate::interact::InteractReq::WithdrawX {
            bank_item_id: ITEM,
            ..
        })
    ));
    assert!(run.transfer_in_flight());
    crate::quester::families::tests::with_tick_bank(&snapshot, None, &mut ledger, 3, |tick| {
        run.interrupt_transfer(tick.actions);
    });
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 4, OpenGate::Open).is_pending());
    snapshot.seed_inventory(
        vec![item(ITEM, "Widget", 2, ItemContainer::Inventory, &["Use"])],
        28,
    );
    open_bank(
        &mut snapshot,
        Some(vec![
            bank_row(ITEM, "Widget", 1),
            bank_row(OTHER, "Other", 1),
        ]),
        Vec::new(),
    );
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 5, OpenGate::Open).is_pending());
    assert!(!run.transfer_in_flight());
    assert!(matches!(run.active, Active::None));
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "the boundary poll must not withdraw the second requested item"
    );
}

#[test]
fn acquire_run_active_match_unchanged() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, bank, banks) = fixture();
    let mut net = item(
        ITEM,
        "Net",
        1,
        ItemContainer::Inventory,
        &["Deposit-1", "Deposit-All"],
    );
    net.slot = 0;
    let mut shrimp = item(
        PRODUCT,
        "Shrimp",
        1,
        ItemContainer::Inventory,
        &["Deposit-1", "Deposit-All"],
    );
    shrimp.slot = 1;
    let mut anchovy = item(
        OTHER,
        "Anchovy",
        1,
        ItemContainer::Inventory,
        &["Deposit-1", "Deposit-All"],
    );
    anchovy.slot = 2;
    snapshot.seed_inventory(vec![net, shrimp, anchovy], 28);

    let mut shrimp_side = side_row(PRODUCT, "Shrimp", 1);
    shrimp_side.slot = 1;
    let mut anchovy_side = side_row(OTHER, "Anchovy", 1);
    anchovy_side.slot = 2;
    open_bank(
        &mut snapshot,
        Some(Vec::new()),
        vec![shrimp_side, anchovy_side],
    );

    let request = request(
        BankStage::Planned,
        Vec::new(),
        Some(DepositSpec::Capacity {
            keep: Arc::from([ITEM]),
            slots: 1,
        }),
        Some(bank),
        false,
        false,
        CloseBank::LeaveOpen,
    );
    let mut ledger = None;
    let mut run = new_run(request, &snapshot, None, &mut ledger, &banks);
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    assert!(
        run.transfer_in_flight(),
        "capacity deposits use Active::Machine"
    );
    assert!(matches!(
        complete_interaction(&mut ledger, 1),
        HostEffect::Interaction(crate::interact::InteractReq::InvButton { id: PRODUCT, .. })
    ));

    crate::quester::families::tests::with_tick_bank(&snapshot, None, &mut ledger, 2, |tick| {
        run.interrupt_transfer(tick.actions)
    });
    assert!(run.transfer_in_flight());

    let mut net = item(
        ITEM,
        "Net",
        1,
        ItemContainer::Inventory,
        &["Deposit-1", "Deposit-All"],
    );
    net.slot = 0;
    let mut anchovy = item(
        OTHER,
        "Anchovy",
        1,
        ItemContainer::Inventory,
        &["Deposit-1", "Deposit-All"],
    );
    anchovy.slot = 2;
    snapshot.seed_inventory(vec![net, anchovy], 28);
    let mut anchovy_side = side_row(OTHER, "Anchovy", 1);
    anchovy_side.slot = 2;
    open_bank(
        &mut snapshot,
        Some(vec![bank_row(PRODUCT, "Shrimp", 1)]),
        vec![anchovy_side],
    );
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 3, OpenGate::Open).is_pending());
    assert!(!run.transfer_in_flight());
    assert!(matches!(run.active, Active::None));
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
}

#[test]
fn wear_only_run_waits_for_worn_evidence_without_walk_or_bank_effects() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, _bank, banks) = fixture();
    snapshot.seed_inventory(
        vec![item(
            ITEM,
            "Widget",
            1,
            ItemContainer::Inventory,
            &["Wield"],
        )],
        28,
    );
    let request = request(
        BankStage::Planned,
        vec![need(
            ITEM,
            "Widget",
            1,
            1,
            Presence::Wielded,
            NeedKind::Required,
            SourceSet::PACK,
        )],
        None,
        None,
        false,
        false,
        CloseBank::LeaveOpen,
    );
    let mut ledger = None;
    let mut run = new_run(request, &snapshot, None, &mut ledger, &banks);

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    assert!(
        matches!(first_effect(&ledger), HostEffect::Interaction(crate::interact::InteractReq::Held { action, .. }) if action.as_str() == "Wield")
    );
    assert!(!ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .any(|action| matches!(
            &action.effect,
            HostEffect::BankPick(_) | HostEffect::Walk(_)
        )));
    assert!(matches!(
        complete_interaction(&mut ledger, 1),
        HostEffect::Interaction(crate::interact::InteractReq::Held { .. })
    ));

    assert!(
        poll_at(&mut run, &snapshot, None, &mut ledger, 2, OpenGate::Open).is_pending(),
        "accepted click is not worn evidence"
    );
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    snapshot.seed_inventory(Vec::new(), 28);
    snapshot.seed_equipment(vec![item(ITEM, "Widget", 1, ItemContainer::Equipment, &[])]);
    let receipt = done(poll_at(
        &mut run,
        &snapshot,
        None,
        &mut ledger,
        3,
        OpenGate::Open,
    ));
    assert!(receipt.complete);
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
}

#[test]
fn child_due_reports_need_index_then_resume_replans_from_new_inventory() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, _bank, banks) = fixture();
    snapshot.seed_inventory(
        vec![item(ITEM, "Widget", 1, ItemContainer::Inventory, &["Use"])],
        28,
    );
    let request = request(
        BankStage::Planned,
        vec![
            need(
                ITEM,
                "Widget",
                1,
                1,
                Presence::Held,
                NeedKind::Required,
                SourceSet::PACK,
            ),
            need(
                OTHER,
                "Component",
                1,
                1,
                Presence::Held,
                NeedKind::Required,
                SourceSet::PACK | SourceSet::CHILD,
            ),
        ],
        None,
        None,
        false,
        false,
        CloseBank::LeaveOpen,
    );
    let mut ledger = None;
    let mut run = new_run(request, &snapshot, None, &mut ledger, &banks);

    assert!(matches!(
        poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open),
        Poll::Ready(Ok(AcquireOutcome::ChildDue { need: 1 }))
    ));
    assert!(ledger
        .as_ref()
        .is_none_or(|ledger| ledger.outbox.is_empty()));
    assert!(matches!(
        poll_at(&mut run, &snapshot, None, &mut ledger, 2, OpenGate::Open),
        Poll::Ready(Ok(AcquireOutcome::ChildDue { need: 1 }))
    ));

    run.resume_after_child();
    snapshot.seed_inventory(
        vec![
            item(ITEM, "Widget", 1, ItemContainer::Inventory, &["Use"]),
            item(OTHER, "Component", 1, ItemContainer::Inventory, &["Use"]),
        ],
        28,
    );
    let receipt = done(poll_at(
        &mut run,
        &snapshot,
        None,
        &mut ledger,
        3,
        OpenGate::Open,
    ));
    assert!(receipt.complete);
    assert!(ledger
        .as_ref()
        .is_none_or(|ledger| ledger.outbox.is_empty()));
}

#[test]
fn explicit_cancel_revokes_queued_effect_and_is_terminal() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (snapshot, bank, banks) = fixture();
    let mut ledger = None;
    let mut run = new_run(
        authored_request(Some(bank), true, Vec::new()),
        &snapshot,
        None,
        &mut ledger,
        &banks,
    );

    crate::quester::families::tests::with_tick_bank(&snapshot, None, &mut ledger, 1, |tick| {
        assert!(run.poll(tick, OpenGate::Open).is_pending());
    });
    assert!(matches!(first_effect(&ledger), HostEffect::BankPick(_)));
    assert!(ledger.as_ref().unwrap().outbox[0].live());
    crate::quester::families::tests::with_tick_bank(&snapshot, None, &mut ledger, 2, |tick| {
        run.cancel(tick.actions)
    });
    assert!(
        !ledger.as_ref().unwrap().outbox[0].live(),
        "cancel must revoke the queued BankPick before host dispatch"
    );
    assert!(matches!(
        poll_at(&mut run, &snapshot, None, &mut ledger, 3, OpenGate::Open),
        Poll::Ready(Err(ActionError::Cancelled))
    ));
    assert!(!ledger.as_ref().unwrap().outbox[0].live());
}

#[test]
fn stale_run_key_cancels_and_prevents_later_dispatch() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (snapshot, bank, banks) = fixture();
    let mut ledger = None;
    let mut run = new_run(
        authored_request(Some(bank), true, Vec::new()),
        &snapshot,
        None,
        &mut ledger,
        &banks,
    );
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    assert!(ledger.as_ref().unwrap().outbox[0].live());

    let stale =
        crate::quester::families::tests::with_tick_bank(&snapshot, None, &mut ledger, 2, |tick| {
            tick.cx.evidence.run.run += 1;
            run.poll(tick, OpenGate::Open)
        });
    assert!(matches!(stale, Poll::Ready(Err(ActionError::Stale))));
    assert!(!ledger.as_ref().unwrap().outbox[0].live());
    assert!(matches!(
        poll_at(&mut run, &snapshot, None, &mut ledger, 3, OpenGate::Open),
        Poll::Ready(Err(ActionError::Cancelled))
    ));
}

#[test]
fn unchanged_pending_poll_uses_zero_allocations_after_warmup() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (snapshot, bank, banks) = fixture();
    let mut ledger = None;
    let mut run = new_run(
        authored_request(Some(bank), true, Vec::new()),
        &snapshot,
        None,
        &mut ledger,
        &banks,
    );

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 2, OpenGate::Open).is_pending());
    let allocations = allocation_counter::measure(|| {
        for tick in 3..=1002 {
            assert!(
                poll_at(&mut run, &snapshot, None, &mut ledger, tick, OpenGate::Open).is_pending()
            );
        }
    });
    assert_eq!(
        allocations,
        allocation_counter::AllocationInfo::default(),
        "unchanged pending polls must allocate nothing after warmup"
    );
    assert!(matches!(first_effect(&ledger), HostEffect::BankPick(_)));
}

#[test]
fn unchanged_blocked_polls_retain_one_block_without_allocations() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, bank, banks) = fixture();
    open_bank(&mut snapshot, Some(Vec::new()), Vec::new());
    let mut ledger = None;
    let request = request(
        BankStage::Planned,
        vec![need(
            ITEM,
            "Widget",
            1,
            1,
            Presence::Held,
            NeedKind::Required,
            SourceSet::PACK | SourceSet::BANK,
        )],
        None,
        Some(bank),
        false,
        false,
        CloseBank::LeaveOpen,
    );
    let mut run = new_run(request, &snapshot, None, &mut ledger, &banks);
    let message = match poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open) {
        Poll::Ready(Err(ActionError::Blocked(message))) => message,
        _ => panic!("the observed empty bank must block the required need"),
    };
    let block = std::ptr::from_ref(run.block.as_deref().expect("one retained block"));
    let allocations = allocation_counter::measure(|| {
        for tick in 2..=1001 {
            match poll_at(&mut run, &snapshot, None, &mut ledger, tick, OpenGate::Open) {
                Poll::Ready(Err(ActionError::Blocked(current))) => {
                    assert!(Arc::ptr_eq(&current, &message));
                }
                _ => panic!("the unchanged block must remain terminal"),
            }
            assert_eq!(run.block.as_deref().map(std::ptr::from_ref), Some(block));
        }
    });
    assert_eq!(allocations, allocation_counter::AllocationInfo::default());
    assert!(ledger
        .as_ref()
        .is_none_or(|ledger| ledger.outbox.is_empty()));
}

#[test]
fn open_attempt_cap_uses_observed_tick_deadlines_and_real_interactions() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (snapshot, bank, banks) = fixture();
    let mut ledger = None;
    let mut run = new_run(
        request(
            BankStage::Planned,
            vec![need(
                ITEM,
                "Tool",
                1,
                1,
                Presence::Held,
                NeedKind::Required,
                SourceSet::PACK | SourceSet::BANK,
            )],
            None,
            Some(bank),
            true,
            false,
            CloseBank::LeaveOpen,
        ),
        &snapshot,
        None,
        &mut ledger,
        &banks,
    );
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    complete_pick(&mut ledger, bank, 1);

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 2, OpenGate::Open).is_pending());
    assert!(is_open_stand(first_effect(&ledger)));
    complete_interaction(&mut ledger, 2);

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 10, OpenGate::Open).is_pending());
    assert!(is_open_stand(first_effect(&ledger)));
    complete_interaction(&mut ledger, 10);

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 18, OpenGate::Open).is_pending());
    assert!(is_open_stand(first_effect(&ledger)));
    complete_interaction(&mut ledger, 18);

    assert!(matches!(
        poll_at(&mut run, &snapshot, None, &mut ledger, 26, OpenGate::Open),
        Poll::Ready(Err(ActionError::Failed(message)))
            if message.as_ref() == "bank-unavailable: bank did not open after 3 attempts"
    ));
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
}

#[test]
fn close_attempt_cap_uses_observed_tick_deadlines_and_real_interactions() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, bank, banks) = fixture();
    open_bank(&mut snapshot, Some(Vec::new()), Vec::new());
    let mut request = authored_request(Some(bank), false, Vec::new());
    request.close = CloseBank::Close;
    let mut ledger = None;
    let mut run = new_run(request, &snapshot, None, &mut ledger, &banks);

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    assert!(is_close(first_effect(&ledger)));
    complete_interaction(&mut ledger, 1);

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 9, OpenGate::Open).is_pending());
    assert!(is_close(first_effect(&ledger)));
    complete_interaction(&mut ledger, 9);

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 17, OpenGate::Open).is_pending());
    assert!(is_close(first_effect(&ledger)));
    complete_interaction(&mut ledger, 17);

    assert!(matches!(
        poll_at(&mut run, &snapshot, None, &mut ledger, 25, OpenGate::Open),
        Poll::Ready(Err(ActionError::Failed(message)))
            if message.as_ref() == "bank-close-failed: bank did not close after 3 attempts"
    ));
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    assert!(crate::quester::families::tests::with_tick_bank(
        &snapshot,
        None,
        &mut ledger,
        26,
        |tick| bank_open(&tick.cx),
    ));
}

#[test]
fn bank_machine_and_acquire_run_size_of_pinned() {
    use std::mem::size_of;
    println!(
        "BankMachine={} AcquireRun={} AcquireRequest={} BankReceipt={} Provisioner={} Gatherer={} Quester={} SurvivalStep={}",
        size_of::<crate::native_bank::BankMachine>(),
        size_of::<AcquireRun>(),
        size_of::<AcquireRequest>(),
        size_of::<BankReceipt>(),
        size_of::<crate::quester::provision::Provisioner>(),
        size_of::<crate::gatherer::Gatherer>(),
        size_of::<crate::quester::runner::Quester>(),
        size_of::<crate::survival::SurvivalStep>(),
    );
    // BankMachine=456 B and AcquireRun=920 B are measured baselines; the
    // 64 B headroom allows shared capacity keep-set plumbing without per-tick state.
    assert!(size_of::<crate::native_bank::BankMachine>() <= 520);
    assert!(size_of::<AcquireRun>() <= 984);
    assert_eq!(size_of::<BankReceipt>(), 8);
    assert!(size_of::<crate::survival::SurvivalStep>() <= 192);
    // Capacity deposits replace the 24-byte Vec with a bounded, allocation-free
    // 28-row snapshot. That adds 208 bytes through the inline bank child.
    assert!(size_of::<crate::quester::provision::Provisioner>() <= 1120);
    assert!(size_of::<AcquireRequest>() <= 232);
}

#[test]
fn bank_walk_prerequisite_handoff_reissues_once_and_cancel_keeps_run_state() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, bank, banks) = fixture();
    let data =
        api::game_data::for_revision(api::selected::ClientRevision::R289).expect("R289 data");
    let catalog = api::selected::FamilyPreparation::run(move |worker| {
        data.prepare_prereq_steps(worker)
            .expect("R289 prerequisite catalog")
    })
    .expect("family preparation worker")
    .join()
    .expect("prerequisite preparation");
    let (_, row) = catalog
        .iter()
        .find(|(_, row)| row.crossings.iter().any(|entry| entry.crossing.option > 0))
        .expect("R289 has a prerequisite crossing");
    let entry = row
        .crossings
        .iter()
        .find(|entry| entry.crossing.option > 0)
        .expect("selected row has a supported operation");
    let crossing = entry.crossing;
    let gate = api::prereq_steps::ActionableGate {
        // The dialogue fixture needs its actual barrier promise, not any journal interval.
        gate: api::selected::QuestGate::Window(row.establishes.clone()),
        truth: api::selected::Truth::Unknown,
        leg: 0,
        crossing,
    };
    let mut player = snapshot
        .local_player()
        .expect("fixture has a local player")
        .clone();
    player.player.network = crossing.takeoff;
    player.player.actor.tile = crossing.takeoff;
    player.player.actor.moving = false;
    snapshot.seed_local_player(player);
    crate::quester::families::tests::seed_prereq_crossing(&mut snapshot, row, &crossing);
    let target = WorldTile {
        x: crossing.at.x + 80,
        z: crossing.at.z + 80,
        level: crossing.at.level,
    };

    let mut ledger = None;
    let mut run = new_run(
        authored_request(Some(bank), true, Vec::new()),
        &snapshot,
        None,
        &mut ledger,
        &banks,
    );
    run.phase = Phase::Walk;
    run.target = Some(target);
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 1, OpenGate::Open).is_pending());
    let (outer_id, outer_run) = {
        let action = &ledger.as_ref().unwrap().outbox[0];
        (action.request_id.get(), action.run())
    };
    ledger.as_mut().unwrap().walk = Some(crate::native::WalkReceipt {
        goal: None,
        request_id: outer_id,
        evidence: api::quest_progress::EvidenceStamp {
            run: outer_run,
            tick: 2,
            sequence: 2,
        },
        end: crate::native::WalkEnd::NeedsStep(Arc::from([gate])),
        blocked: None,
        detail: None,
        refusal: None,
        assessment: None,
        escape: None,
        prereq_allow: Some(crate::native::PrereqAllow {
            dialogue: true,
            quest_start: true,
        }),
    });
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 2, OpenGate::Open).is_pending());
    let (probe_authority, probe_id, probe_run) = {
        let probe = ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .find(|action| {
                matches!(
                    &action.effect,
                    HostEffect::Interaction(crate::interact::InteractReq::Loc {
                        id: Some(id),
                        ..
                    }) if *id == crossing.loc_id
                )
            })
            .expect("prerequisite probe was submitted");
        (probe.authority(), probe.request_id.get(), probe.run())
    };
    ledger.as_mut().unwrap().complete_interaction(
        &probe_authority,
        crate::native::InteractionReceipt {
            request_id: probe_id,
            evidence: api::quest_progress::EvidenceStamp {
                run: probe_run,
                tick: 2,
                sequence: 2,
            },
            accepted: true,
            chat_since: 0,
        },
    );
    let mut player = snapshot
        .local_player()
        .expect("fixture has a local player")
        .clone();
    player.player.network = crossing.to;
    player.player.actor.tile = crossing.to;
    snapshot.seed_local_player(player);
    snapshot.seed_chat_lines(vec![api::snapshot::ChatLineView {
        sequence: 1,
        type_: 0,
        username: None,
        text: entry.pass.message.to_string(),
    }]);
    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 3, OpenGate::Open).is_pending());

    let walk_ids: Vec<_> = ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .filter_map(|action| match &action.effect {
            HostEffect::Walk(request) => Some((action.request_id.get(), request.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(walk_ids.len(), 1, "only the live reissue remains queued");
    let (reissued_id, reissued) = &walk_ids[0];
    assert!(
        *reissued_id > outer_id,
        "the outer request was replaced once"
    );
    assert_eq!(reissued.target, target);
    assert_eq!(reissued.radius, 0);
    assert_eq!(reissued.required_after.tick, 3);
    assert!(reissued.cross.is_empty());
    assert!(reissued.avoid.is_empty());
    assert!(!reissued.protect);
    assert!(!reissued.food_guard);
    assert_eq!(run.target, Some(target));
    assert_eq!(run.run, outer_run);

    assert!(poll_at(&mut run, &snapshot, None, &mut ledger, 4, OpenGate::Open).is_pending());
    assert_eq!(
        ledger.as_ref().unwrap().outbox.len(),
        1,
        "pending wrapper polls keep only their one live reissue"
    );
    crate::quester::families::tests::with_tick_bank(&snapshot, None, &mut ledger, 5, |tick| {
        run.cancel(tick.actions)
    });
    assert!(!ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .find(|action| action.request_id.get() == *reissued_id)
        .expect("reissued walk remains recorded")
        .live());
    assert_eq!(
        run.target,
        Some(target),
        "cancel preserves the selected bank target"
    );
    assert!(matches!(
        poll_at(&mut run, &snapshot, None, &mut ledger, 6, OpenGate::Open),
        Poll::Ready(Err(ActionError::Cancelled))
    ));
    assert_eq!(run.target, Some(target));
}
