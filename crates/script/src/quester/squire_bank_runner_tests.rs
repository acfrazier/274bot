//! Runner-level pins for the two Knight's Sword bank steps the operator's
//! headed runs tripped on (design-bank-snapshot §2.4, D4):
//!
//! - `squire-coin-float`: `withdraw` 300 coins, `partial_ok`, settle needs
//!   100 held. A bank already seen empty this session refuses the trip at
//!   once with the counts; a hint or an unknown bank still makes the one
//!   verifying trip.
//! - `withdraw-portrait`: skip `has portrait OR (bank_known AND NOT bank_has
//!   portrait)`. A bank opened empty makes `bank_known` true and `bank_has`
//!   false at that same open bank, so the skip fires with no park.
use super::*;
use crate::native::{HostEffect, InteractionReceipt, NativeOutput, ScriptStatus};
use crate::quester::families::tests::with_tick_output_bank;
use crate::quester::path::{PathDocument, PredicateDocument, StepDocument};
use api::bank_memory::{BankMemory, Origin};
use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer, ItemView, LocLayer, LocView};

#[derive(Default)]
struct Capture {
    statuses: Vec<ScriptStatus>,
    logs: Vec<String>,
}

impl NativeOutput for Capture {
    fn status(&mut self, status: ScriptStatus) {
        self.statuses.push(status);
    }
    fn paint(&mut self, _: Arc<crate::shim::ScriptPaint>) {}
    fn log(&mut self, _: api::hostlog::Level, message: &str) {
        self.logs.push(message.to_owned());
    }
    fn settings_applied(&mut self, _: u64) {}
}

fn fact(kind: &str, args: serde_json::Value) -> PredicateDocument {
    PredicateDocument::Fact {
        kind: kind.into(),
        version: 1,
        args,
    }
}

fn step(
    id: &str,
    kind: &str,
    args: serde_json::Value,
    skip_if: PredicateDocument,
    settle: PredicateDocument,
) -> StepDocument {
    StepDocument {
        id: FactKey::new(id),
        kind: kind.to_owned(),
        version: 1,
        args,
        comment: None,
        advances: Some(false),
        skip_if,
        settle,
    }
}

/// `squire-coin-float` as authored in `paths/289/squire.json`, minus its
/// `stage_in` guard (a foreign quest here).
fn coin_float_step() -> StepDocument {
    step(
        "squire-coin-float",
        "bank",
        serde_json::json!({
            "op": "withdraw",
            "at": "nearest",
            "items": [{"obj": "coins", "qty": 300}],
            "partial_ok": true
        }),
        fact(
            "item_count_at_least",
            serde_json::json!({"obj": "coins", "qty": 100}),
        ),
        fact(
            "item_count_at_least",
            serde_json::json!({"obj": "coins", "qty": 100}),
        ),
    )
}

/// `withdraw-portrait` as authored in `paths/289/squire.json`.
fn withdraw_portrait_step() -> StepDocument {
    step(
        "withdraw-portrait",
        "bank",
        serde_json::json!({
            "op": "withdraw",
            "at": "nearest",
            "items": [{"obj": "knights_portrait", "qty": 1}],
            "partial_ok": true
        }),
        PredicateDocument::Any(vec![
            fact("has_item", serde_json::json!({"obj": "knights_portrait"})),
            PredicateDocument::All(vec![
                fact("bank_known", serde_json::json!({})),
                PredicateDocument::Not(Box::new(fact(
                    "bank_has",
                    serde_json::json!({"obj": "knights_portrait"}),
                ))),
            ]),
        ]),
        PredicateDocument::All(vec![]),
    )
}

fn wait_step(id: &str) -> StepDocument {
    step(
        id,
        "wait",
        serde_json::json!({"until":{"Any":[]},"max_ticks":1000}),
        PredicateDocument::Any(vec![]),
        PredicateDocument::All(vec![]),
    )
}

fn bank_tile() -> api::WorldTile {
    api::WorldTile {
        x: 3092,
        z: 3242,
        level: 0,
    }
}

struct Fixture {
    script: Quester,
    snapshot: GameSnapshot,
    ledger: Option<Box<crate::native::ledger::Ledger>>,
    /// The account's bank memory, filled the way the host fills it.
    bank: BankMemory,
    named: api::named_banks::NamedBank,
    /// Rows the fixture bank shows once a trip opens it.
    opens_with: Vec<ItemView>,
    picks: u32,
    opens: u32,
}

impl Fixture {
    fn new(
        prelude: Vec<StepDocument>,
        steps: Vec<StepDocument>,
        bank: BankMemory,
        opens_with: Vec<ItemView>,
    ) -> Self {
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let quests = Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap());
        let mut document: PathDocument = super::super::compile::decode_cook().unwrap();
        document.quest.as_mut().unwrap().owns_inventory = true;
        document.roles[0].prelude = prelude;
        document.roles[0].sequences[0].steps = steps;
        let path =
            super::super::compile::compile_uncached_for_test(&document, &data, &quests).unwrap();
        let named = api::named_banks::NamedBank::new("Squire fixture bank", bank_tile());
        let mut script = Quester::new(
            RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            path,
            Arc::clone(&data),
            quests,
            Arc::new(api::named_banks::NamedBankFacts::from_banks(vec![named])),
        );
        let stage = script.path.colour_not_started.clone();
        script.seq_index =
            sequence_for_stage(&script.path, stage.0.as_ref()).expect("not-started sequence");
        script.stage = Some(stage);
        script.needs_read = false;
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_quest_statuses(
            vec![api::snapshot::QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 42,
                colour: 0xf80000,
            }],
            true,
        );
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_equipment(Vec::new());
        snapshot.seed_local_player(super::super::families::tests::local_player(bank_tile()));
        snapshot.seed_locs(vec![LocView {
            id: 2213,
            name: Some("Bank booth".into()),
            actions: vec![Some("Use-quickly".into())],
            tile: bank_tile(),
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
        Self {
            script,
            snapshot,
            ledger: None,
            bank,
            named,
            opens_with,
            picks: 0,
            opens: 0,
        }
    }

    /// One scripted tick with the host's observe before it, then the host
    /// side of any bank selection or stand open the tick queued.
    fn drive(&mut self, tick: u64, output: &mut Capture) -> ScriptFlow {
        self.bank.track(&self.snapshot, tick);
        let flow = with_tick_output_bank(
            &self.snapshot,
            Some(&self.bank),
            &mut self.ledger,
            tick,
            output,
            |native| self.script.tick(native).unwrap(),
        );
        let Some(ledger) = self.ledger.as_mut() else {
            return flow;
        };
        let Some(action) = ledger.outbox.first() else {
            return flow;
        };
        match &action.effect {
            HostEffect::BankPick(_) => {
                let action = ledger.outbox.remove(0);
                let authority = action.authority();
                ledger.complete_bank_pick(
                    &authority,
                    crate::bank::BankPickReceipt {
                        request_id: authority.request_id().get(),
                        evidence: api::quest_progress::EvidenceStamp {
                            run: authority.run(),
                            tick,
                            sequence: tick,
                        },
                        selected: crate::bank::SelectedBank {
                            bank_index: 0,
                            access_tile: bank_tile(),
                            kind: crate::bank::PickKind::Reachable,
                            access: Some(Arc::new(crate::bank::BankStandAccess {
                                bank: self.named,
                                stand_tile: bank_tile(),
                                kind: crate::bank::AccessKind::Booth,
                                stand_op: 1,
                                name: None,
                                choose: None,
                            })),
                        },
                    },
                );
                self.picks += 1;
            }
            HostEffect::Interaction(crate::shim::InteractReq::OpenStand { .. }) => {
                let action = ledger.outbox.remove(0);
                let authority = action.authority();
                ledger.complete_interaction(
                    &authority,
                    InteractionReceipt {
                        request_id: authority.request_id().get(),
                        evidence: api::quest_progress::EvidenceStamp {
                            run: authority.run(),
                            tick,
                            sequence: tick,
                        },
                        accepted: true,
                        chat_since: 0,
                    },
                );
                self.snapshot.seed_bank_observation(
                    1,
                    tick,
                    Some(self.opens_with.clone()),
                    Vec::new(),
                );
                self.opens += 1;
            }
            _ => {}
        }
        flow
    }

    fn current_step(&self) -> Option<&str> {
        self.script.current_step().map(|step| step.id.0.as_ref())
    }
}

fn portrait(data: &api::game_data::SelectedGameData, container: ItemContainer) -> ItemView {
    let item = data.item_by_alias("knights_portrait").unwrap();
    ItemView {
        def: api::obj_names::ItemDefView {
            id: item.id,
            name: Some("Portrait".into()),
            stackable: false,
            members: false,
            base_value: 0,
            noted: false,
            certificate_link: -1,
            certificate_template: -1,
        },
        container,
        action_family: if container == ItemContainer::Bank {
            ItemActionFamily::Component
        } else {
            ItemActionFamily::Held
        },
        slot: 0,
        count: 1,
        actions: vec![Some("Withdraw-1".into())],
        component_id: if container == ItemContainer::Bank {
            7
        } else {
            3214
        },
    }
}

/// D4: a bank seen empty this session refuses the coin float before any
/// selection or walk, with the counts, and the run parks on it instead of
/// burning settle timeouts.
#[test]
fn coin_float_from_a_bank_seen_empty_this_session_fails_at_once() {
    let mut fixture = Fixture::new(
        vec![coin_float_step()],
        vec![wait_step("after-float")],
        BankMemory::seeded(&[], Origin::Session),
        Vec::new(),
    );
    let mut output = Capture::default();
    for tick in 1..=4 {
        fixture.drive(tick, &mut output);
        if fixture.script.parked {
            break;
        }
    }
    assert!(
        fixture.script.parked,
        "a session-known shortage parks at once"
    );
    assert_eq!(fixture.picks, 0, "no bank selection");
    assert_eq!(fixture.opens, 0, "no bank open");
    let message = fixture.script.blocked_failure().message;
    assert!(
        message.contains("need 300 Coins; held 0, banked 0"),
        "the shortage names the counts: {message}"
    );
    assert!(
        !output
            .logs
            .iter()
            .any(|line| line.contains("step settle timeout")),
        "no settle timeout is spent: {:?}",
        output.logs
    );
}

/// The same shortage from a hint or an unknown bank is advisory: exactly one
/// verifying trip begins (D4), and nothing parks before the bank is seen.
#[test]
fn coin_float_from_a_hint_or_unknown_bank_makes_the_one_verifying_trip() {
    for memory in [BankMemory::seeded(&[], Origin::Hint), BankMemory::default()] {
        let origin = memory.origin();
        let mut fixture = Fixture::new(
            vec![coin_float_step()],
            vec![wait_step("after-float")],
            memory,
            Vec::new(),
        );
        let mut output = Capture::default();
        for tick in 1..=12 {
            fixture.drive(tick, &mut output);
            if fixture.opens > 0 {
                // The host observes the table it opened on the next frame.
                fixture.drive(tick + 1, &mut output);
                break;
            }
        }
        assert_eq!(fixture.picks, 1, "{origin:?}: one bank selection");
        assert_eq!(fixture.opens, 1, "{origin:?}: one bank open");
        assert!(
            !fixture.script.parked,
            "{origin:?}: the trip is the verification"
        );
        assert_eq!(
            fixture.bank.origin(),
            Origin::Session,
            "{origin:?}: the open bank was observed"
        );
    }
}

/// A bank opened empty makes `bank_known` true and `bank_has portrait` false
/// at that same open bank: the skip fires, no park, one trip.
#[test]
fn portrait_skip_fires_once_the_bank_is_opened_without_a_portrait() {
    let mut fixture = Fixture::new(
        Vec::new(),
        vec![withdraw_portrait_step(), wait_step("after-portrait")],
        BankMemory::default(),
        Vec::new(),
    );
    let mut output = Capture::default();
    for tick in 1..=64 {
        fixture.drive(tick, &mut output);
        if fixture.current_step() == Some("after-portrait") {
            break;
        }
    }
    assert_eq!(
        fixture.current_step(),
        Some("after-portrait"),
        "the portrait step is skipped at the open empty bank"
    );
    assert!(!fixture.script.parked, "no watchdog park");
    assert_eq!(fixture.picks, 1, "one trip learns the bank");
    assert_eq!(fixture.opens, 1);
    assert_eq!(fixture.bank.origin(), Origin::Session);
    assert!(output.logs.iter().any(|line| {
        line.contains("step withdraw-portrait skipped") && line.contains("evaluated true")
    }));
}

/// With the portrait in the bank the step is selected (the bank is known
/// and holds it); with the portrait held it is skipped without any trip.
#[test]
fn portrait_step_is_selected_when_banked_and_skipped_when_held() {
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let banked = portrait(&data, ItemContainer::Bank);
    let mut fixture = Fixture::new(
        Vec::new(),
        vec![withdraw_portrait_step(), wait_step("after-portrait")],
        BankMemory::default(),
        vec![banked],
    );
    let mut output = Capture::default();
    for tick in 1..=64 {
        fixture.drive(tick, &mut output);
        if fixture.current_step() == Some("withdraw-portrait") && fixture.bank.known() {
            break;
        }
    }
    assert_eq!(fixture.current_step(), Some("withdraw-portrait"));
    assert!(!fixture.script.parked);
    assert_eq!(fixture.picks, 1);

    let portrait_id = data.item_by_alias("knights_portrait").unwrap().id;
    let mut held = Fixture::new(
        Vec::new(),
        vec![withdraw_portrait_step(), wait_step("after-portrait")],
        BankMemory::default(),
        Vec::new(),
    );
    held.snapshot
        .seed_inventory(vec![portrait(&data, ItemContainer::Inventory)], 28);
    let mut output = Capture::default();
    for tick in 1..=8 {
        held.drive(tick, &mut output);
        if held.current_step() == Some("after-portrait") {
            break;
        }
    }
    assert_eq!(held.current_step(), Some("after-portrait"));
    assert_eq!(held.picks, 0, "a held portrait needs no bank at all");
    assert_eq!(
        held.bank.count(portrait_id),
        None,
        "the bank was never opened: still unknown"
    );
}

/// design-bank-snapshot §4 F10 / §8 point 3: the authored scan pattern
/// (`bank {op: scan}` guarded by `skip_if: bank_known`, as every shipped
/// Path writes it) costs no trip whenever the memory is known, `Hint`
/// included; with an unknown memory `bank_known` is `Unknown` and the
/// runner's one provisioning scan learns the bank, after which the step
/// is skipped — one trip either way, never a second.
#[test]
fn authored_scan_completes_without_a_trip_when_the_memory_is_known() {
    let scan = step(
        "scan-bank",
        "bank",
        serde_json::json!({"op": "scan", "at": "nearest"}),
        fact("bank_known", serde_json::json!({})),
        fact("bank_known", serde_json::json!({})),
    );
    for origin in [Origin::Hint, Origin::Session] {
        let mut fixture = Fixture::new(
            Vec::new(),
            vec![scan.clone(), wait_step("after-scan")],
            BankMemory::seeded(&[], origin),
            Vec::new(),
        );
        let mut output = Capture::default();
        for tick in 1..=12 {
            fixture.drive(tick, &mut output);
            if fixture.current_step() == Some("after-scan") {
                break;
            }
        }
        assert_eq!(fixture.current_step(), Some("after-scan"), "{origin:?}");
        assert_eq!(
            fixture.picks, 0,
            "{origin:?}: a known memory answers the scan"
        );
        assert!(!fixture.script.parked);
    }

    let mut fixture = Fixture::new(
        Vec::new(),
        vec![scan, wait_step("after-scan")],
        BankMemory::default(),
        Vec::new(),
    );
    let mut output = Capture::default();
    for tick in 1..=64 {
        fixture.drive(tick, &mut output);
        if fixture.current_step() == Some("after-scan") {
            break;
        }
    }
    assert_eq!(fixture.current_step(), Some("after-scan"));
    assert_eq!(fixture.picks, 1, "an unknown memory is learnt by one trip");
    assert_eq!(fixture.bank.origin(), Origin::Session);
}

/// An unguarded `bank {op: scan}` step begun against a known memory
/// completes on its first poll with no selection, walk or open queued.
#[test]
fn unguarded_scan_step_run_completes_at_once_when_the_memory_is_known() {
    let scan = step(
        "scan-bank",
        "bank",
        serde_json::json!({"op": "scan", "at": "nearest"}),
        PredicateDocument::Any(vec![]),
        fact("bank_known", serde_json::json!({})),
    );
    for origin in [Origin::Hint, Origin::Session] {
        let fixture = Fixture::new(
            Vec::new(),
            vec![scan.clone(), wait_step("after-scan")],
            BankMemory::seeded(&[], origin),
            Vec::new(),
        );
        let plan =
            Arc::clone(&fixture.script.path.sequences[fixture.script.seq_index].steps[0].plan);
        let mut ledger = None;
        let poll = crate::quester::families::tests::with_tick_bank(
            &fixture.snapshot,
            Some(&fixture.bank),
            &mut ledger,
            1,
            |tick| {
                let required_after = tick.cx.evidence();
                let mut cx = StepContext {
                    tick,
                    quests: &fixture.script.quests,
                    progress: &[],
                    required_after,
                    banks: &fixture.script.banks,
                    choices: &fixture.script.choices,
                };
                let mut run = plan.begin(&mut cx).expect("scan step begins");
                run.poll(&mut cx)
            },
        );
        assert!(
            matches!(poll, Poll::Ready(Ok(_))),
            "{origin:?}: a known memory answers the scan at once"
        );
        assert!(
            ledger
                .as_ref()
                .is_none_or(|ledger| ledger.outbox.is_empty()),
            "{origin:?}: no bank selection, walk or open"
        );
    }
}
