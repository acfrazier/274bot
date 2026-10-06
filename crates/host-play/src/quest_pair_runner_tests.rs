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
fn paired_path(
    path_id: &str,
    gang: Gang,
    selected: &Arc<api::game_data::SelectedGameData>,
    quests: &Arc<api::quest_facts::QuestCatalog>,
) -> Arc<CompiledPath> {
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
    runner.slot.on_game_tick(&mut ctx);
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
fn runner_fails_fast_when_peer_is_bound_to_hero() {
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
    let status = alice.slot.native_status().expect("Quester status");
    let failure = status
        .failure
        .as_ref()
        .expect("different Path parks promptly");
    assert_eq!(
        failure.message.as_ref(),
        "partner is running a different Path; select the same paired Path on both accounts"
    );
    assert!(
        !seats[0].busy() && !seats[1].busy(),
        "different Paths reserve no phase"
    );
    assert!(
        !seats[0].waiting(alice.run),
        "different Paths do not leave an admission waiter"
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
