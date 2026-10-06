use super::*;
use api::interact::Driver;
use api::prot::Out;
use api::selected::{ClientRevision, FactKey, Knowledge, RunKey};
use api::snapshot::{GameSnapshot, QuestStatusView};
use api::RandomClaim;
use script::native::{Interrupt, NativeTick, Script, ScriptFailure, ScriptFlow, StopReason};
use script::quester::compile::{compile_path_for_gang, CompiledPath};
use script::quester::pair::{
    AccountKey, Gang, PairBinding, PairFrame, PairRegistration, PairSettings, PartnerDeclaration,
    PartnerRole, QuestPairPort,
};
use script::quester::runner::Quester;
use script::{CompiledTick, RunState, ScriptCtx, SlotScript};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Sink;
impl Out for Sink {
    fn p1_enc(&mut self, _: i32) {}
    fn p1(&mut self, _: i32) {}
    fn p2(&mut self, _: i32) {}
    fn p4(&mut self, _: i32) {}
    fn pjstr(&mut self, _: &str) {}
}

#[derive(Default)]
struct NoopDriver {
    out: Sink,
}
impl Driver for NoopDriver {
    fn revision(&self) -> ClientRevision {
        ClientRevision::R289
    }
    fn set_menu(&mut self, _: i32, _: i32, _: i32, _: i32, _: i32) {}
    fn do_action(&mut self, _: i32) -> bool {
        false
    }
    fn try_move(
        &mut self,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: bool,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
    ) -> bool {
        false
    }
    fn local_route(&self) -> Option<(i32, i32)> {
        None
    }
    fn build_base(&self) -> (i32, i32) {
        (0, 0)
    }
    fn loc_typecode(&self, _: i32, _: i32) -> Option<i32> {
        None
    }
    fn out(&mut self) -> &mut dyn Out {
        &mut self.out
    }
    fn login(&mut self, _: &str, _: &str, _: bool) -> bool {
        false
    }
}

/// A test-only outer script publishes the normal per-account settings while
/// delegating every tick and lifecycle callback to the production Quester.
struct QuesterHarness {
    path: Arc<CompiledPath>,
    settings: PairSettings,
    control: Arc<Mutex<Option<Quester>>>,
}
impl Script for QuesterHarness {
    fn pair_settings(&self) -> Option<PairSettings> {
        Some(self.settings.clone())
    }

    fn pair_binding(&self) -> Option<PairBinding<'_>> {
        let declaration = self.path.partner.as_ref()?;
        Some(PairBinding {
            path: &self.path.id,
            protocol: &declaration.protocol,
            digest: &self.path.digest,
            role: self.path.role.as_ref()?,
        })
    }

    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        self.control
            .lock()
            .unwrap()
            .as_mut()
            .expect("installed Quester")
            .tick(tick)
    }

    fn interrupt(&mut self, event: Interrupt) {
        self.control
            .lock()
            .unwrap()
            .as_mut()
            .expect("installed Quester")
            .interrupt(event);
    }

    fn on_random(&mut self, event: &api::DetectedRandom) -> RandomClaim {
        self.control
            .lock()
            .unwrap()
            .as_mut()
            .expect("installed Quester")
            .on_random(event)
    }

    fn on_stop(&mut self, reason: StopReason) {
        self.control
            .lock()
            .unwrap()
            .as_mut()
            .expect("installed Quester")
            .on_stop(reason);
    }
}

struct Runner {
    slot: SlotScript,
    port: Arc<dyn QuestPairPort>,
    path: Arc<CompiledPath>,
    control: Arc<Mutex<Option<Quester>>>,
    settings: PairSettings,
    run: RunKey,
}

fn selected_and_quests() -> (
    Arc<api::game_data::SelectedGameData>,
    Arc<api::quest_facts::QuestCatalog>,
) {
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests =
        Arc::new(api::quest_facts::QuestCatalog::from_identity(selected.quest_identity()).unwrap());
    (selected, quests)
}

fn rewrite_progress_path(value: &mut serde_json::Value, path_id: &str) {
    match value {
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                if key == "quest" && value.as_str() == Some("vampire") {
                    *value = serde_json::Value::String(path_id.to_owned());
                } else {
                    rewrite_progress_path(value, path_id);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                rewrite_progress_path(value, path_id);
            }
        }
        _ => {}
    }
}
/// Reuse Vampire's runnable content, rebinding authored progress predicates to
/// the requested Path identity before compiling its paired roles.
fn paired_document(
    path_id: &str,
    quests: &api::quest_facts::QuestCatalog,
) -> script::quester::path::PathDocument {
    let mut source: serde_json::Value =
        serde_json::from_str(include_str!("../../script/paths/289/vampire.json")).unwrap();
    rewrite_progress_path(&mut source, path_id);
    source
        .as_object_mut()
        .unwrap()
        .insert("id".into(), serde_json::Value::String(path_id.to_owned()));
    let mut document: script::quester::path::PathDocument = serde_json::from_value(source).unwrap();
    let binding = quests
        .quest(path_id)
        .ok()
        .and_then(|facts| match &facts.progress_binding {
            Knowledge::Known(binding) => Some(binding.clone()),
            _ => None,
        })
        .unwrap_or_else(|| document.roles[0].progress_binding.clone());
    document.id = FactKey::new(path_id);
    document.display_name = quests.quest(path_id).unwrap().display.to_string();
    document.partner = Some(PartnerDeclaration {
        protocol: FactKey::new("arrav"),
        roles: [
            PartnerRole {
                id: FactKey::new("phoenix"),
                gang: Gang::Phoenix,
            },
            PartnerRole {
                id: FactKey::new("blackarm"),
                gang: Gang::BlackArm,
            },
        ],
    });
    let mut phoenix = document.roles[0].clone();
    phoenix.role = Some(FactKey::new("phoenix"));
    phoenix.progress_binding = binding.clone();
    let mut blackarm = document.roles[0].clone();
    blackarm.role = Some(FactKey::new("blackarm"));
    blackarm.progress_binding = binding;
    document.roles = vec![phoenix, blackarm];
    document
}

fn paired_path(
    path_id: &str,
    gang: Gang,
    selected: &Arc<api::game_data::SelectedGameData>,
    quests: &Arc<api::quest_facts::QuestCatalog>,
) -> Arc<CompiledPath> {
    let document = paired_document(path_id, quests);
    let bytes = serde_json::to_vec(&document).unwrap();
    script::quester::compile::prepare_for_test({
        let selected = Arc::clone(selected);
        let quests = Arc::clone(quests);
        move |worker| compile_path_for_gang(&bytes, &selected, &quests, worker, Some(gang))
    })
    .unwrap_or_else(|error| panic!("compile synthetic paired {path_id} Path: {error:?}"))
}

fn broker_seats() -> [Arc<dyn QuestPairPort>; 2] {
    let core = Arc::new(QuestPairCoordinator::default());
    core.remember("alice");
    core.remember("bob");
    [
        core.seat("alice", "127.0.0.1", 44594),
        core.seat("bob", "127.0.0.1", 44594),
    ]
}

fn start_runner(
    port: Arc<dyn QuestPairPort>,
    incarnation: u64,
    path: Arc<CompiledPath>,
    partner: &str,
    gang: Gang,
    selected: &Arc<api::game_data::SelectedGameData>,
    quests: &Arc<api::quest_facts::QuestCatalog>,
) -> Runner {
    let settings = PairSettings {
        partner: Some(AccountKey(Arc::from(partner))),
        gang: Some(gang),
    };
    let control = Arc::new(Mutex::new(None));
    let mut slot = SlotScript::new();
    slot.bind_incarnation(incarnation);
    slot.bind_quest_pairs(Arc::clone(&port));
    slot.start_test_script(
        Box::new(QuesterHarness {
            path: Arc::clone(&path),
            settings: settings.clone(),
            control: Arc::clone(&control),
        }),
        Some(Arc::clone(selected)),
    )
    .unwrap();
    let run = slot.native_run().expect("compiled Quester run");
    *control.lock().unwrap() = Some(Quester::new(
        run,
        Arc::clone(&path),
        Arc::clone(selected),
        Arc::clone(quests),
        Arc::new(api::named_banks::NamedBankFacts::empty()),
    ));
    Runner {
        slot,
        port,
        path,
        control,
        settings,
        run,
    }
}

fn restart_runner(
    runner: &mut Runner,
    selected: &Arc<api::game_data::SelectedGameData>,
    quests: &Arc<api::quest_facts::QuestCatalog>,
) {
    let control = Arc::new(Mutex::new(None));
    runner
        .slot
        .start_test_script(
            Box::new(QuesterHarness {
                path: Arc::clone(&runner.path),
                settings: runner.settings.clone(),
                control: Arc::clone(&control),
            }),
            Some(Arc::clone(selected)),
        )
        .unwrap();
    runner.run = runner.slot.native_run().expect("restarted Quester run");
    *control.lock().unwrap() = Some(Quester::new(
        runner.run,
        Arc::clone(&runner.path),
        Arc::clone(selected),
        Arc::clone(quests),
        Arc::new(api::named_banks::NamedBankFacts::empty()),
    ));
    runner.control = control;
}

/// Mirror the broker's normal ready observations and an owned Arrav journal
/// receipt before the script tick, so the test isolates paired admission.
fn prime_runner(runner: &Runner, observe_arrav_gang: bool) {
    let declaration = runner.path.partner.as_ref().unwrap();
    let role = runner.path.role.as_ref().unwrap();
    let binding = PairBinding {
        path: &runner.path.id,
        protocol: &declaration.protocol,
        digest: &runner.path.digest,
        role,
    };
    let pin = pin();
    for tick in [1, 3] {
        runner.port.observe(
            PairRegistration {
                run: runner.run,
                pin: Arc::clone(&pin),
                settings: Some(runner.settings.clone()),
                ready: true,
                evidence: stamp(runner.run, tick),
            },
            PairFrame {
                binding: Some(binding),
                inventory: None,
            },
        );
    }
    if observe_arrav_gang {
        runner
            .port
            .observe_gang(&read(runner.run))
            .expect("fresh Arrav journal belongs to ready run");
    }
}

fn not_started_snapshot(path_id: &str, quests: &api::quest_facts::QuestCatalog) -> GameSnapshot {
    let facts = quests.quest(path_id).unwrap();
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: facts.display.to_string(),
            component_id: 1,
            colour: 0xf80000,
        }],
        true,
    );
    snapshot
}

fn run_tick(
    runner: &mut Runner,
    selected: &api::game_data::SelectedGameData,
    snapshot: &GameSnapshot,
    tick: u64,
) {
    run_slot_tick(&mut runner.slot, selected, snapshot, tick);
}

fn run_slot_tick(
    slot: &mut SlotScript,
    selected: &api::game_data::SelectedGameData,
    snapshot: &GameSnapshot,
    tick: u64,
) {
    let mut driver = NoopDriver::default();
    let compiled = CompiledTick {
        selected: Some(selected),
        ..CompiledTick::default()
    };
    let mut ctx = ScriptCtx {
        driver: &mut driver,
        tick,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: Some(snapshot),
        obj_names: None,
        compiled,
    };
    slot.on_game_tick(&mut ctx);
}

fn assert_admission_watchdog_waits(runner: &mut Runner) {
    let now = Instant::now();
    for now in [now, now + script::watchdog::WEDGE + Duration::from_secs(1)] {
        assert_eq!(
            runner.slot.feed_watchdog(
                now,
                None,
                &[],
                false,
                true,
                &[script::shim::InteractReq::LoopSettled],
            ),
            script::WatchdogAction::None,
            "gameplay recovery must not race the bounded admission deadline"
        );
    }
    assert!(runner.port.waiting(runner.run));
}

fn waiting_for_admission(runner: &Runner) -> bool {
    runner
        .slot
        .native_status()
        .is_some_and(|status| {
            status.failure.is_none()
                && status.fields.iter().any(|field| {
                    field.key == "waiting_for"
                        && field.label == "Partner admission"
                        && matches!(&field.value, script::native::StatusValue::Text(text) if text.as_ref() == runner.path.id.0.as_ref())
                })
        })
}

fn no_admission_wait(runner: &Runner) -> bool {
    runner.slot.native_status().is_some_and(|status| {
        status.failure.is_none()
            && !status.fields.iter().any(|field| {
                field.key == "waiting_for"
                    && field.label == "Partner admission"
                    && matches!(&field.value, script::native::StatusValue::Text(text) if text.as_ref() == runner.path.id.0.as_ref())
            })
    })
}

#[test]
fn runner_admission_waits_for_late_peer_start() {
    let (selected, quests) = selected_and_quests();
    let paths = [
        paired_path("blackarmgang", Gang::Phoenix, &selected, &quests),
        paired_path("blackarmgang", Gang::BlackArm, &selected, &quests),
    ];
    let seats = broker_seats();
    let snapshot = not_started_snapshot("blackarmgang", &quests);
    let mut alice = start_runner(
        Arc::clone(&seats[0]),
        1,
        Arc::clone(&paths[0]),
        "bob",
        Gang::Phoenix,
        &selected,
        &quests,
    );
    prime_runner(&alice, true);

    run_tick(&mut alice, &selected, &snapshot, 4);
    assert!(
        waiting_for_admission(&alice),
        "the first Start must wait for an unregistered peer instead of parking"
    );
    assert!(
        !seats[0].busy(),
        "a missing peer must not reserve a broker phase"
    );
    assert_admission_watchdog_waits(&mut alice);

    let mut bob = start_runner(
        Arc::clone(&seats[1]),
        2,
        Arc::clone(&paths[1]),
        "alice",
        Gang::BlackArm,
        &selected,
        &quests,
    );
    prime_runner(&bob, true);
    run_tick(&mut bob, &selected, &snapshot, 4);
    run_tick(&mut alice, &selected, &snapshot, 5);
    run_tick(&mut bob, &selected, &snapshot, 5);

    assert!(
        no_admission_wait(&alice),
        "late reciprocal Start completes admission"
    );
    assert!(
        no_admission_wait(&bob),
        "both started roles complete admission"
    );
    assert!(
        !seats[0].busy() && !seats[1].busy(),
        "completed admission releases its lease"
    );
}

#[test]
fn runner_admission_waits_for_stopped_peer_to_restart() {
    let (selected, quests) = selected_and_quests();
    let paths = [
        paired_path("blackarmgang", Gang::Phoenix, &selected, &quests),
        paired_path("blackarmgang", Gang::BlackArm, &selected, &quests),
    ];
    let seats = broker_seats();
    let snapshot = not_started_snapshot("blackarmgang", &quests);
    let mut alice = start_runner(
        Arc::clone(&seats[0]),
        1,
        Arc::clone(&paths[0]),
        "bob",
        Gang::Phoenix,
        &selected,
        &quests,
    );
    let mut bob = start_runner(
        Arc::clone(&seats[1]),
        2,
        Arc::clone(&paths[1]),
        "alice",
        Gang::BlackArm,
        &selected,
        &quests,
    );
    prime_runner(&alice, true);
    prime_runner(&bob, true);

    let stopped_run = bob.run;
    bob.slot.stop();
    bob.port.invalidate(stopped_run);
    run_tick(&mut alice, &selected, &snapshot, 4);
    assert!(
        waiting_for_admission(&alice),
        "NotReady from a stopped peer is a bounded wait, not an immediate park"
    );
    assert!(!seats[0].busy(), "NotReady must not reserve a broker phase");
    assert_admission_watchdog_waits(&mut alice);

    restart_runner(&mut bob, &selected, &quests);
    assert_ne!(
        bob.run, stopped_run,
        "Stop/Start creates a fresh run identity"
    );
    prime_runner(&bob, true);
    run_tick(&mut bob, &selected, &snapshot, 4);
    run_tick(&mut alice, &selected, &snapshot, 5);
    run_tick(&mut bob, &selected, &snapshot, 5);
    assert!(
        no_admission_wait(&alice),
        "restarted peer completes admission"
    );
    assert!(
        no_admission_wait(&bob),
        "both started roles complete admission"
    );
    assert!(
        !seats[0].busy() && !seats[1].busy(),
        "completed admission releases its lease"
    );
}

#[test]
fn runner_waits_without_reservation_when_peer_is_bound_to_hero() {
    let (selected, quests) = selected_and_quests();
    let arrav = paired_path("blackarmgang", Gang::Phoenix, &selected, &quests);
    let hero = paired_path("hero", Gang::BlackArm, &selected, &quests);
    let seats = broker_seats();
    let snapshot = not_started_snapshot("blackarmgang", &quests);
    let mut alice = start_runner(
        Arc::clone(&seats[0]),
        1,
        arrav,
        "bob",
        Gang::Phoenix,
        &selected,
        &quests,
    );
    let bob = start_runner(
        Arc::clone(&seats[1]),
        2,
        hero,
        "alice",
        Gang::BlackArm,
        &selected,
        &quests,
    );
    prime_runner(&alice, true);
    prime_runner(&bob, false);

    run_tick(&mut alice, &selected, &snapshot, 4);
    assert!(
        waiting_for_admission(&alice),
        "a different queue row waits for the matching Path"
    );
    assert!(
        !seats[0].busy() && !seats[1].busy(),
        "different Paths reserve no phase"
    );
    assert_admission_watchdog_waits(&mut alice);
    assert!(
        !seats[1].waiting(bob.run),
        "the different-Path peer remains independent"
    );
}

#[test]
fn runner_keeps_pending_pair_admission_through_hold_and_random_until_pause() {
    let (selected, quests) = selected_and_quests();
    let paths = [
        paired_path("blackarmgang", Gang::Phoenix, &selected, &quests),
        paired_path("blackarmgang", Gang::BlackArm, &selected, &quests),
    ];
    let seats = broker_seats();
    let snapshot = not_started_snapshot("blackarmgang", &quests);
    let mut alice = start_runner(
        Arc::clone(&seats[0]),
        1,
        Arc::clone(&paths[0]),
        "bob",
        Gang::Phoenix,
        &selected,
        &quests,
    );
    let _bob = start_runner(
        Arc::clone(&seats[1]),
        2,
        Arc::clone(&paths[1]),
        "alice",
        Gang::BlackArm,
        &selected,
        &quests,
    );
    prime_runner(&alice, true);
    prime_runner(&_bob, true);

    run_tick(&mut alice, &selected, &snapshot, 4);
    assert!(
        seats[0].waiting(alice.run),
        "the first role owns a pending reservation"
    );
    assert!(
        seats[0].busy() && seats[1].busy(),
        "broker fences both registered roles"
    );

    alice
        .control
        .lock()
        .unwrap()
        .as_mut()
        .expect("installed Quester")
        .interrupt(Interrupt::Hold(true));
    assert!(
        seats[0].waiting(alice.run),
        "Hold preserves the pending admission"
    );
    alice
        .control
        .lock()
        .unwrap()
        .as_mut()
        .expect("installed Quester")
        .interrupt(Interrupt::Hold(false));
    assert!(
        seats[0].waiting(alice.run),
        "releasing Hold preserves the pending admission"
    );

    assert_eq!(
        alice.slot.on_random(&api::DetectedRandom {
            kind: api::random::RandomKind::Dialog,
            name: "test guardian dialog".into(),
            ours: true,
            npc_index: None,
        }),
        RandomClaim::Host
    );
    assert!(
        seats[0].waiting(alice.run),
        "random handling retains the reserved phase"
    );
    assert!(
        seats[0].busy() && seats[1].busy(),
        "random handling does not cancel the peer lease"
    );

    alice.slot.pause();
    assert!(
        !seats[0].busy() && !seats[1].busy(),
        "explicit Pause still cancels the reservation"
    );
    assert_eq!(alice.slot.state(), RunState::Paused);
}

#[test]
fn quest_pair_r2_runner_mismatched_queue_waits_for_solo_row_without_reserving_peer() {
    use script::quester::queue::{Queue, QueueSettings};
    use script::quester::registry::{FolderSource, PathRegistry};
    use script::quester::runner::QueuedQuester;

    let (selected, quests) = selected_and_quests();
    let folder = std::env::temp_dir().join(format!("quest-pair-queue-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("blackarmgang.json"),
        serde_json::to_vec(&paired_document("blackarmgang", &quests)).unwrap(),
    )
    .unwrap();
    let registry = script::quester::compile::prepare_for_test({
        let selected = Arc::clone(&selected);
        let quests = Arc::clone(&quests);
        let folder = folder.clone();
        move |worker| {
            PathRegistry::load(
                &FolderSource {
                    enabled: true,
                    folder,
                },
                &selected,
                &quests,
                worker,
            )
            .unwrap()
        }
    });
    std::fs::remove_dir_all(folder).unwrap();
    let queue = Queue::from_registry(
        registry,
        QueueSettings {
            quests: vec!["cook".into(), "blackarmgang".into()],
            order_override: vec!["cook".into(), "blackarmgang".into()],
            partner_account: Some(AccountKey(Arc::from("alice"))),
            gang: Some(Gang::BlackArm),
            ..Default::default()
        },
    )
    .unwrap();
    let seats = broker_seats();
    let mut alice = start_runner(
        Arc::clone(&seats[0]),
        1,
        paired_path("blackarmgang", Gang::Phoenix, &selected, &quests),
        "bob",
        Gang::Phoenix,
        &selected,
        &quests,
    );
    prime_runner(&alice, true);
    let mut bob = SlotScript::new();
    bob.bind_incarnation(2);
    bob.bind_quest_pairs(Arc::clone(&seats[1]));
    bob.start_test_script(
        Box::new(QueuedQuester::new(
            RunKey {
                slot: 0,
                run: 0,
                session: 0,
            },
            Arc::clone(&selected),
            Arc::clone(&quests),
            Arc::new(api::named_banks::NamedBankFacts::empty()),
            queue,
        )),
        Some(Arc::clone(&selected)),
    )
    .unwrap();
    let bob_run = bob.native_run().unwrap();
    let mut snapshot = not_started_snapshot("blackarmgang", &quests);
    snapshot.seed_quest_statuses(
        vec![
            QuestStatusView {
                name: quests.quest("cook").unwrap().display.to_string(),
                component_id: 2,
                colour: 0x00f800,
            },
            QuestStatusView {
                name: quests.quest("blackarmgang").unwrap().display.to_string(),
                component_id: 1,
                colour: 0xf80000,
            },
        ],
        true,
    );
    snapshot.seed_inventory(vec![], 28);
    snapshot.seed_main_modal(-1, vec![]);
    snapshot.seed_chat_modal(-1, vec![]);
    // Bob prepares the earlier solo row. His run exists without a pair binding.
    run_slot_tick(&mut bob, &selected, &snapshot, 3);
    run_tick(&mut alice, &selected, &snapshot, 4);
    assert!(waiting_for_admission(&alice));
    assert!(
        !seats[0].busy() && !seats[1].busy(),
        "an earlier solo queue row must not be reserved by the waiting pair"
    );
    assert_admission_watchdog_waits(&mut alice);
    assert!(
        !seats[1].waiting(bob_run),
        "the solo peer's watchdog stays live"
    );
    // The completed solo row advances through the real queue into Arrav.
    for tick in 5u64..20_000 {
        run_slot_tick(&mut bob, &selected, &snapshot, tick);
        while let Some(action) = bob.take_native_action() {
            let authority = action.authority();
            match action.effect {
                script::native::HostEffect::Interaction(script::shim::InteractReq::IfButton {
                    component_id: 1,
                }) => snapshot.seed_main_modal(
                    8134,
                    vec![
                        journal_widget(
                            8144,
                            quests
                                .quest("blackarmgang")
                                .unwrap()
                                .journal_title
                                .as_ref()
                                .unwrap(),
                        ),
                        journal_widget(8145, &read(bob_run).lines[0]),
                    ],
                ),
                script::native::HostEffect::Interaction(script::shim::InteractReq::CloseModal) => {
                    snapshot.seed_main_modal(-1, vec![]);
                }
                _ => panic!("unexpected solo-to-pair journal input"),
            }
            bob.complete_native_interaction(
                &authority,
                script::native::InteractionReceipt {
                    request_id: action.request_id.get(),
                    evidence: stamp(bob_run, tick),
                    accepted: true,
                    chat_since: 0,
                },
            );
        }
        run_tick(&mut alice, &selected, &snapshot, tick);
        if no_admission_wait(&alice)
            && bob.native_status().is_some_and(|status| {
                status.fields.iter().any(|field| {
                    field.key == "quest_id"
                        && matches!(&field.value, script::native::StatusValue::Text(id) if id.as_ref() == "blackarmgang")
                })
            })
            && !seats[0].busy()
            && !seats[1].busy()
        {
            assert_eq!(bob.native_run(), Some(bob_run), "queue advance is not Stop/Start");
            assert!(seats[1].settings(bob_run).is_ok(), "the unjoined peer run stays usable");
            run_slot_tick(&mut bob, &selected, &snapshot, tick + 1);
            assert!(bob.native_status().unwrap().fields.iter().all(|field| {
                field.key != "waiting_for" || field.label != "Partner admission"
            }));
            return;
        }
        std::thread::yield_now();
    }
    panic!(
        "the mismatched real queue did not reach reciprocal admission: {:?}",
        bob.native_status()
    );
}

fn journal_widget(component_id: i32, text: &str) -> api::snapshot::WidgetView {
    use api::snapshot::{WidgetKind, WidgetRoot, WidgetView};
    WidgetView {
        kind: WidgetKind::Widget,
        component_id,
        layer_id: 0,
        parent_id: 0,
        root_component_id: 8134,
        root: WidgetRoot::Main,
        type_: 4,
        button_type: 0,
        client_code: 0,
        x: 0,
        y: 0,
        width: 0,
        height: 0,
        scroll_height: 0,
        scroll_position: 0,
        hidden: false,
        text: Some(text.into()),
        alternate_text: None,
        button_text: None,
        target_verb: None,
        target_base: None,
        target_mask: 0,
        model_type: 0,
        model_id: 0,
        alternate_model_type: 0,
        alternate_model_id: 0,
        scripts: None,
        script_comparators: None,
        script_operands: None,
        varp_bindings: Vec::new(),
        colour: 0,
        actions: Vec::new(),
        items: Vec::new(),
    }
}

fn phase_path(
    gang: Gang,
    selected: &Arc<api::game_data::SelectedGameData>,
    quests: &Arc<api::quest_facts::QuestCatalog>,
) -> Arc<CompiledPath> {
    let mut document = paired_document("blackarmgang", quests);
    document.quest.as_mut().unwrap().owns_inventory = true;
    for (side, role) in document.roles.iter_mut().enumerate() {
        role.prelude.clear();
        let step = &mut role.sequences[0].steps[0];
        step.kind = "partner".into();
        step.args = serde_json::json!({
            "phase": "arrav:test",
            "give": [{"obj": if side == 0 { "egg" } else { "pot_flour" }, "qty": 1}],
            "take": [{"obj": if side == 0 { "pot_flour" } else { "egg" }, "qty": 1}],
            "rendezvous": {"tile": [3208, 3216, 0], "source": "test rendezvous"}
        });
        step.advances = Some(false);
        step.skip_if = script::quester::path::PredicateDocument::Any(vec![]);
        step.settle = script::quester::path::PredicateDocument::All(vec![]);
        let mut wait = step.clone();
        wait.id = FactKey::new("wait-before-pair");
        wait.kind = "wait".into();
        let has_egg = script::quester::path::PredicateDocument::Fact {
            kind: "has_item".into(),
            version: 1,
            args: serde_json::json!({"obj": "egg"}),
        };
        wait.args = serde_json::json!({"until": has_egg, "max_ticks": 100});
        wait.skip_if = has_egg;
        role.sequences[0].steps.insert(0, wait);
    }
    let bytes = serde_json::to_vec(&document).unwrap();
    script::quester::compile::prepare_for_test({
        let selected = Arc::clone(selected);
        let quests = Arc::clone(quests);
        move |worker| compile_path_for_gang(&bytes, &selected, &quests, worker, Some(gang))
    })
    .unwrap()
}

fn observe_runner_ready(runner: &Runner, ready: bool, tick: u64) {
    runner.port.observe(
        PairRegistration {
            run: runner.run,
            pin: pin(),
            settings: Some(runner.settings.clone()),
            ready,
            evidence: stamp(runner.run, tick),
        },
        PairFrame {
            binding: Some(PairBinding {
                path: &runner.path.id,
                protocol: &runner.path.partner.as_ref().unwrap().protocol,
                digest: &runner.path.digest,
                role: runner.path.role.as_ref().unwrap(),
            }),
            inventory: None,
        },
    );
}

#[test]
fn quest_pair_r2_runner_phase_begin_waits_through_partner_hold() {
    let (selected, quests) = selected_and_quests();
    let seats = broker_seats();
    let mut alice = start_runner(
        Arc::clone(&seats[0]),
        1,
        phase_path(Gang::Phoenix, &selected, &quests),
        "bob",
        Gang::Phoenix,
        &selected,
        &quests,
    );
    let mut bob = start_runner(
        Arc::clone(&seats[1]),
        2,
        phase_path(Gang::BlackArm, &selected, &quests),
        "alice",
        Gang::BlackArm,
        &selected,
        &quests,
    );
    prime_runner(&alice, true);
    prime_runner(&bob, true);
    let mut snapshot = not_started_snapshot("blackarmgang", &quests);
    snapshot.seed_inventory(vec![], 28);
    run_tick(&mut alice, &selected, &snapshot, 4);
    run_tick(&mut bob, &selected, &snapshot, 4);
    run_tick(&mut alice, &selected, &snapshot, 5);
    assert!(no_admission_wait(&alice));
    assert!(!seats[0].busy() && !seats[1].busy());
    observe_runner_ready(&bob, false, 5);
    snapshot.seed_inventory(
        vec![held(selected.item_by_alias("egg").unwrap().id, 1, 0)],
        28,
    );
    for tick in 6..=14 {
        run_tick(&mut alice, &selected, &snapshot, tick);
        assert!(
            alice.slot.native_status().unwrap().failure.is_none(),
            "phase-begin holds must wait instead of consuming five failed attempts"
        );
    }
    assert!(!seats[0].busy() && !seats[1].busy());
    assert_admission_watchdog_waits(&mut alice);
    observe_runner_ready(&bob, true, 15);
    observe_runner_ready(&bob, true, 16);
    let mut journal = read(bob.run);
    journal.acquired = stamp(bob.run, 15);
    journal.closed = stamp(bob.run, 16);
    seats[1].observe_gang(&journal).unwrap();
    run_tick(&mut alice, &selected, &snapshot, 15);
    assert!(
        seats[0].busy() && seats[1].busy(),
        "the actor begins the real phase when the peer becomes ready"
    );
    for tick in 16..=19 {
        run_tick(&mut bob, &selected, &snapshot, tick);
    }
    assert!(alice.slot.native_status().unwrap().failure.is_none());
    assert!(bob.slot.native_status().unwrap().failure.is_none());
}
