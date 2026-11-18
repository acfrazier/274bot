//! The shared live pump ends a scenario whose started script ended Failed.
//! The slot is real: a native test script blocks the way the Quester queue
//! does, so the slot stops itself and keeps its failed lifecycle receipt.

use std::time::Duration;

use client::client::{Client, ClientConfig};
use client::dash3d::ClientPlayer;
use client::io::ServerProt;
use scenario::{Proof, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait};
use script::native::{NativeTick, Script, ScriptFailure, ScriptFlow};

use super::{RunnerHandle, RunnerStatus, StartPlan};

const SLOT: &str = "alice";
const PEER: &str = "bob";
const WATCH: &str = "watch the queued Imp Catcher completion";
const NATIVE_REASON: &str = "search_result=Refusal; NoWayOut: ends inside Lesser demon (L82); resolve the condition, then start the Quester again";
/// The runner waits on its final watch, the second of two steps.
const ON_THE_FINAL_WATCH: RunnerStatus = RunnerStatus::Running { step: 1, total: 2 };
/// The runner waits on its final proof after the only step.
const ON_THE_FINAL_PROOF: RunnerStatus = RunnerStatus::Running { step: 1, total: 1 };

/// Ends its run on the first tick with the flow it was given.
struct EndsRun(ScriptFlow);

impl Script for EndsRun {
    fn tick(&mut self, _: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        Ok(self.0.clone())
    }
}

fn blocked(reason: &str) -> ScriptFlow {
    ScriptFlow::Blocked(ScriptFailure {
        code: "queue-blocked".into(),
        message: reason.into(),
    })
}

/// The nightly queue's shape: Start the card, then watch a quest completion
/// for far longer than these tests run.
fn queue_scenario() -> scenario::Scenario {
    scenario::Scenario {
        name: "queue-failure",
        seed: Seed {
            profiles: vec![(SLOT, SLOT)],
            mainland: false,
        },
        steps: vec![
            Step {
                name: "start the catalog card",
                kind: StepKind::StartScript,
                wait: Wait {
                    arm: Proof::Stat { id: 16, min: 0 },
                    budget_ticks: 1,
                },
            },
            Step {
                name: WATCH,
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: Proof::QuestDone {
                        name: "Imp Catcher",
                    },
                    budget_ticks: 24_000,
                },
            },
        ],
        proof: Proof::QuestDone {
            name: "Imp Catcher",
        },
        companions: vec![],
        settings: ScenarioSettings {
            deadline: Duration::from_secs(7200),
            ..ScenarioSettings::default()
        },
    }
}

/// A second owned account, a companion whose script the scenario also starts.
fn with_peer(mut scenario: scenario::Scenario) -> scenario::Scenario {
    scenario.seed.profiles.push((PEER, PEER));
    scenario.companions.push(scenario::Companion {
        profile: 1,
        per_frame: Box::new(|_| false),
    });
    scenario
}

/// One Start step, then a final proof that `Rig::prove` makes true: the proof
/// can become true in the same frame in which a script has already failed.
fn with_stat_proof(mut scenario: scenario::Scenario) -> scenario::Scenario {
    scenario.steps.truncate(1);
    scenario.proof = Proof::Stat { id: 16, min: 2 };
    scenario
}

/// A client that is in the game on a settled scene, with no connection.
fn synthetic_client() -> Client {
    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: std::env::temp_dir()
            .join("absent-pump-cache")
            .to_string_lossy()
            .into_owned(),
        members: true,
        lowmem: true,
    });
    client.ingame = true;
    client.scene_state = 2;
    client.map_build_base_x = 3200;
    client.map_build_base_z = 3200;
    client.local_player = Some(ClientPlayer::at(20, 20));
    client
}

/// A live runner whose staged Starts are already accepted and settled, one for
/// each owned account, over a play with no connection and a synthetic client.
struct Rig {
    handle: RunnerHandle,
    play: host_play::Play,
    client: Client,
    slot_tick: u64,
}

impl Rig {
    /// `None` where the generated game data the native test script needs is
    /// absent (hosted CI).
    fn with(scenario: scenario::Scenario) -> Option<Self> {
        if !api::test_support::game_data_available() {
            return None;
        }
        let cards = scenario
            .seed
            .profiles
            .iter()
            .map(|(slot, _)| {
                let mut card = host_play::live_start::PendingCatalogStart::compiled(
                    *slot,
                    script::CompiledId("Quester"),
                    serde_json::Map::new(),
                );
                card.started = true;
                card.settled = true;
                card
            })
            .collect();
        let play = crate::test_support::empty_play(&host_play::PlayOptions {
            host: "127.0.0.1".into(),
            transport: host_play::Transport::Tcp,
            port: 43594,
            cache_dir: std::env::temp_dir()
                .join("absent-pump-cache")
                .to_string_lossy()
                .into_owned(),
            lowmem: true,
            mainland: false,
        });
        let mut runner = ScenarioRunner::with_world(scenario, None);
        runner.set_scene_settle(Duration::ZERO);
        let handle = RunnerHandle::default();
        handle.install_scenario_runner_for_test(runner);
        handle.install_play(&play);
        handle.stage(StartPlan(cards));
        let mut client = synthetic_client();
        client.runenergy = 0;
        Some(Self {
            handle,
            play,
            client,
            slot_tick: 0,
        })
    }

    fn new() -> Option<Self> {
        Self::with(queue_scenario())
    }

    /// Start a native test script on `slot`, as the pump's accepted Start does.
    fn start_on(&self, slot: &str, flow: ScriptFlow) -> api::selected::RunKey {
        self.play
            .script_start_handle()
            .start_test_script(slot, Box::new(EndsRun(flow)), None)
            .expect("the test script starts")
    }

    fn start(&self, flow: ScriptFlow) -> api::selected::RunKey {
        self.start_on(SLOT, flow)
    }

    /// One client frame on `slot`, which the runner drives or owns as companion.
    fn frame_on(&mut self, slot: &str) {
        self.handle.frame(
            &mut self.client,
            slot,
            host_play::SlotFrameInput::default(),
            ScenarioRunner::tick_with_hold,
            ScenarioRunner::companion_tick,
        );
    }

    /// One client frame on the driven slot, with one observed game tick.
    fn frame(&mut self) {
        self.client.bump_gens(ServerProt::PLAYER_INFO);
        self.frame_on(SLOT);
    }

    fn frames(&mut self, count: usize) {
        for _ in 0..count {
            self.frame();
        }
    }

    /// One observed game tick on `slot`: the script runs its tick.
    fn slot_tick_on(&mut self, slot: &str) {
        self.slot_tick += 1;
        self.play
            .script_start_handle()
            .tick_test_script(slot, self.slot_tick, &mut self.client);
    }

    fn slot_tick(&mut self) {
        self.slot_tick_on(SLOT);
    }

    /// Make the `with_stat_proof` proof true at the next frame.
    fn prove(&mut self) {
        self.client.runenergy = 2;
        self.client.bump_gens(ServerProt::UPDATE_RUNENERGY);
    }

    fn status(&self) -> RunnerStatus {
        self.handle.status().expect("an installed runner")
    }

    fn receipt_state_of(&self, slot: &str) -> Option<script::ScriptTerminalState> {
        self.play
            .script_lifecycle_receipt(slot)
            .map(|receipt| receipt.state)
    }

    fn receipt_state(&self) -> Option<script::ScriptTerminalState> {
        self.receipt_state_of(SLOT)
    }

    fn failure(&self) -> String {
        self.handle.failure_message().expect("the failure message")
    }
}

#[test]
fn key_scenario_binds_only_its_driven_slots_native_input() {
    use api::native_input::{GestureKey, KeyPhase};
    for (driven, other) in [(SLOT, PEER), (PEER, SLOT)] {
        let mut scenario = queue_scenario();
        scenario.seed.profiles = vec![(driven, driven)];
        scenario.steps = vec![Step {
            name: "hold the driven slot's arrow",
            kind: StepKind::KeyGesture {
                key: GestureKey::ArrowLeft,
                duration_ms: 1000,
            },
            wait: Wait {
                arm: Proof::IngameScene2,
                budget_ticks: 100,
            },
        }];
        scenario.proof = Proof::IngameScene2;
        let Some(mut rig) = Rig::with(scenario) else {
            return;
        };
        rig.start_on(driven, ScriptFlow::Continue);
        rig.start_on(other, ScriptFlow::Continue);
        let start = rig.play.script_start_handle();
        let own_input = start.native_input(driven).unwrap();
        let other_input = start.native_input(other).unwrap();
        assert!(!std::sync::Arc::ptr_eq(&own_input, &other_input));
        rig.frame_on(other);
        assert!(own_input.lock().pending_key().is_none());
        for _ in 0..8 {
            rig.client.bump_gens(ServerProt::PLAYER_INFO);
            rig.frame_on(driven);
            if own_input.lock().pending_key().is_some() {
                break;
            }
        }
        let (ticket, key) = own_input
            .lock()
            .pending_key()
            .expect("the production frame must bind the driven slot's native authority");
        assert_eq!(key, GestureKey::ArrowLeft);
        assert_eq!(
            own_input
                .lock()
                .consume_key(std::time::Instant::now(), true),
            Some((ticket, GestureKey::ArrowLeft))
        );
        assert_eq!(
            own_input.key_progress(ticket).unwrap().phase,
            KeyPhase::Held
        );
        rig.frame_on(other);
        assert!(other_input
            .lock()
            .consume_key(std::time::Instant::now(), true)
            .is_none());
        assert!(other_input.key_progress(ticket).is_none());
        assert!(matches!(rig.status(), RunnerStatus::Running { .. }));
    }
}

#[test]
fn a_blocked_queue_fails_the_scenario_while_the_final_watch_is_pending() {
    let Some(mut rig) = Rig::new() else {
        return;
    };
    rig.start(blocked(NATIVE_REASON));
    rig.frames(3);
    assert_eq!(
        rig.status(),
        ON_THE_FINAL_WATCH,
        "the final QuestDone watch is pending"
    );

    rig.slot_tick();
    assert_eq!(rig.play.script_state(SLOT), script::RunState::Idle);
    assert_eq!(
        rig.receipt_state(),
        Some(script::ScriptTerminalState::Failed)
    );
    rig.frame();

    assert_eq!(rig.status(), RunnerStatus::Failed);
    let message = rig.failure();
    assert!(message.contains(NATIVE_REASON), "{message}");
    assert!(message.contains(WATCH), "{message}");
    assert!(
        !message.contains("deadline") && !message.contains("not seen within"),
        "the run fails on the script's failure, not on a clock: {message}"
    );
    let receipt: serde_json::Value =
        serde_json::from_str(&rig.handle.evidence_json().expect("a terminal receipt")).unwrap();
    assert_eq!(receipt["outcome"], "FAIL");
    assert!(
        receipt["message"]
            .as_str()
            .is_some_and(|text| text.contains(NATIVE_REASON)),
        "{receipt}"
    );
}

#[test]
fn a_failed_receipt_of_an_earlier_run_is_not_this_runs_failure() {
    let Some(mut rig) = Rig::new() else {
        return;
    };
    let earlier = rig.start(blocked("the earlier run blocked"));
    rig.slot_tick();
    assert_eq!(
        rig.receipt_state(),
        Some(script::ScriptTerminalState::Failed)
    );

    rig.frames(3);
    assert_eq!(
        rig.status(),
        ON_THE_FINAL_WATCH,
        "the slot still holds the earlier run's failed receipt"
    );

    let later = rig.start(blocked(NATIVE_REASON));
    assert!(later.run > earlier.run);
    rig.slot_tick();
    rig.frame();
    assert_eq!(rig.status(), RunnerStatus::Failed);
    let message = rig.failure();
    assert!(message.contains(NATIVE_REASON), "{message}");
    assert!(!message.contains("the earlier run blocked"), "{message}");
}

#[test]
fn a_paused_stopped_or_completed_script_does_not_end_the_scenario() {
    let Some(mut rig) = Rig::new() else {
        return;
    };
    let run = rig.start(ScriptFlow::Continue);
    rig.frames(3);
    assert_eq!(rig.status(), ON_THE_FINAL_WATCH);

    assert!(rig.play.script_native_pause(SLOT, run, true));
    assert_eq!(rig.play.script_state(SLOT), script::RunState::Paused);
    rig.frames(3);
    assert_eq!(rig.status(), ON_THE_FINAL_WATCH, "a paused script runs on");
    assert!(rig.play.script_native_pause(SLOT, run, false));

    rig.play.script_start_handle().stop(SLOT).unwrap();
    assert_eq!(
        rig.receipt_state(),
        Some(script::ScriptTerminalState::Cancelled)
    );
    rig.frames(3);
    assert_eq!(
        rig.status(),
        ON_THE_FINAL_WATCH,
        "an operator Stop is not a failure"
    );

    rig.start(ScriptFlow::Complete);
    rig.slot_tick();
    assert_eq!(
        rig.receipt_state(),
        Some(script::ScriptTerminalState::Completed)
    );
    rig.frames(3);
    assert_eq!(
        rig.status(),
        ON_THE_FINAL_WATCH,
        "a completed run leaves the watch to the game"
    );

    rig.start(blocked(NATIVE_REASON));
    rig.slot_tick();
    rig.frame();
    assert_eq!(
        rig.status(),
        RunnerStatus::Failed,
        "the same pump still ends the scenario on a failure"
    );
}

#[test]
fn a_failed_script_is_not_hidden_by_the_frame_that_proves_the_scenario() {
    let Some(mut rig) = Rig::with(with_stat_proof(queue_scenario())) else {
        return;
    };
    rig.start(blocked(NATIVE_REASON));
    rig.frames(3);
    assert_eq!(
        rig.status(),
        ON_THE_FINAL_PROOF,
        "the Start step finished and the final proof is pending"
    );

    rig.slot_tick();
    assert_eq!(
        rig.receipt_state(),
        Some(script::ScriptTerminalState::Failed)
    );
    rig.prove();
    rig.frame();

    assert_eq!(
        rig.status(),
        RunnerStatus::Failed,
        "the script failed before the frame that proves the scenario"
    );
    let message = rig.failure();
    assert!(message.contains(NATIVE_REASON), "{message}");
    assert!(message.contains(SLOT), "{message}");
}

#[test]
fn a_companions_failed_script_ends_the_scenario_from_its_own_frame() {
    let Some(mut rig) = Rig::with(with_peer(queue_scenario())) else {
        return;
    };
    rig.start(ScriptFlow::Continue);
    rig.start_on(PEER, blocked(NATIVE_REASON));
    rig.frames(3);
    assert_eq!(rig.status(), ON_THE_FINAL_WATCH);

    rig.slot_tick_on(PEER);
    assert_eq!(
        rig.receipt_state_of(PEER),
        Some(script::ScriptTerminalState::Failed)
    );
    rig.frame_on(PEER);

    assert_eq!(
        rig.status(),
        RunnerStatus::Failed,
        "a failure of the second owned script must not wait for the scenario cap"
    );
    let message = rig.failure();
    assert!(message.contains(NATIVE_REASON), "{message}");
    assert!(message.contains(PEER), "{message}");
}

#[test]
fn a_companions_failed_script_is_not_hidden_by_the_frame_that_proves_the_scenario() {
    let Some(mut rig) = Rig::with(with_peer(with_stat_proof(queue_scenario()))) else {
        return;
    };
    rig.start(ScriptFlow::Continue);
    rig.start_on(PEER, blocked(NATIVE_REASON));
    rig.frames(3);
    assert_eq!(rig.status(), ON_THE_FINAL_PROOF);

    rig.slot_tick_on(PEER);
    rig.prove();
    rig.frame();

    assert_eq!(
        rig.status(),
        RunnerStatus::Failed,
        "the companion's script failed before the frame that proves the scenario"
    );
    let message = rig.failure();
    assert!(message.contains(NATIVE_REASON), "{message}");
    assert!(message.contains(PEER), "{message}");
}

#[test]
fn a_companions_failed_receipt_of_an_earlier_run_is_not_this_runs_failure() {
    let Some(mut rig) = Rig::with(with_peer(queue_scenario())) else {
        return;
    };
    rig.start_on(PEER, blocked("the earlier peer run blocked"));
    rig.slot_tick_on(PEER);
    assert_eq!(
        rig.receipt_state_of(PEER),
        Some(script::ScriptTerminalState::Failed)
    );

    // The companion's next Start is accepted but its run is not installed yet:
    // the slot still holds the failed receipt of the earlier run.
    rig.start(ScriptFlow::Continue);
    rig.frames(3);
    rig.frame_on(PEER);
    assert_eq!(
        rig.receipt_state_of(PEER),
        Some(script::ScriptTerminalState::Failed)
    );
    assert_eq!(
        rig.status(),
        ON_THE_FINAL_WATCH,
        "the companion's slot holds the failed receipt of its earlier run"
    );

    // The later run fails under a newer generation: that failure counts.
    rig.start_on(PEER, blocked(NATIVE_REASON));
    rig.slot_tick_on(PEER);
    rig.frame_on(PEER);
    assert_eq!(rig.status(), RunnerStatus::Failed);
    let message = rig.failure();
    assert!(message.contains(NATIVE_REASON), "{message}");
    assert!(!message.contains("the earlier peer run"), "{message}");
}

#[test]
fn each_started_script_is_read_under_its_own_start_generation() {
    let Some(mut rig) = Rig::with(with_peer(queue_scenario())) else {
        return;
    };
    // The driven slot is on its third Start; the companion's slot is on its
    // first. A failure of the companion's run carries the lower generation.
    for _ in 0..2 {
        rig.start(ScriptFlow::Continue);
        rig.play.script_start_handle().stop(SLOT).unwrap();
    }
    let driven = rig.start(ScriptFlow::Continue);
    let peer = rig.start_on(PEER, blocked(NATIVE_REASON));
    assert!(driven.run > peer.run, "{driven:?} {peer:?}");
    rig.frames(3);
    assert_eq!(rig.status(), ON_THE_FINAL_WATCH);

    rig.slot_tick_on(PEER);
    rig.frame_on(PEER);
    assert_eq!(
        rig.status(),
        RunnerStatus::Failed,
        "the driven slot's higher generation must not hide the companion's failure"
    );
    assert!(rig.failure().contains(NATIVE_REASON));
}

#[test]
fn a_companions_failure_published_before_the_pass_is_recorded_cannot_pass() {
    let Some(mut rig) = Rig::with(with_peer(with_stat_proof(queue_scenario()))) else {
        return;
    };
    rig.start(ScriptFlow::Continue);
    let peer_run = rig.start_on(PEER, blocked(NATIVE_REASON));
    rig.frames(3);
    assert_eq!(rig.status(), ON_THE_FINAL_PROOF);
    rig.frame_on(PEER);
    rig.prove();

    // The companion's own thread publishes its Failed receipt after the pump
    // read the receipts and before the runner checks its final proof. The tick
    // closure runs between the two: the read has returned when it starts.
    let handle = rig.play.script_start_handle();
    let (read_done, read_done_rx) = std::sync::mpsc::sync_channel(0);
    let (published, published_rx) = std::sync::mpsc::sync_channel(0);
    std::thread::scope(|scope| {
        let companion = scope.spawn(move || {
            let mut peer_client = synthetic_client();
            read_done_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("the driven frame reaches the runner tick");
            handle.tick_test_script(PEER, 1, &mut peer_client);
            assert_eq!(
                handle.failure_since(PEER, peer_run.run).as_deref(),
                Some(NATIVE_REASON),
                "the companion thread publishes the native Failed receipt"
            );
            published.send(()).unwrap();
        });
        rig.handle.frame(
            &mut rig.client,
            SLOT,
            host_play::SlotFrameInput::default(),
            |runner, client, hold| {
                read_done.send(()).unwrap();
                published_rx
                    .recv_timeout(Duration::from_secs(5))
                    .expect("the companion publishes while the driven frame holds the runner");
                ScenarioRunner::tick_with_hold(runner, client, hold);
            },
            ScenarioRunner::companion_tick,
        );
        companion.join().expect("the companion thread finishes");
    });

    assert_eq!(
        rig.receipt_state_of(PEER),
        Some(script::ScriptTerminalState::Failed)
    );
    assert_eq!(
        rig.status(),
        RunnerStatus::Failed,
        "the failure was published before the PASS was recorded"
    );
    let message = rig.failure();
    assert!(message.contains(NATIVE_REASON), "{message}");
    assert!(message.contains(PEER), "{message}");
    let receipt: serde_json::Value =
        serde_json::from_str(&rig.handle.evidence_json().expect("a terminal receipt")).unwrap();
    assert_eq!(receipt["outcome"], "FAIL");

    rig.frame_on(PEER);
    rig.frame();
    assert_eq!(
        rig.status(),
        RunnerStatus::Failed,
        "later frames keep the verdict"
    );
}

#[test]
fn started_scripts_that_did_not_fail_let_the_scenario_pass() {
    let Some(mut rig) = Rig::with(with_peer(with_stat_proof(queue_scenario()))) else {
        return;
    };
    rig.start(ScriptFlow::Continue);
    rig.start_on(PEER, ScriptFlow::Complete);
    rig.frames(3);
    assert_eq!(rig.status(), ON_THE_FINAL_PROOF);
    rig.slot_tick_on(PEER);
    assert_eq!(
        rig.receipt_state_of(PEER),
        Some(script::ScriptTerminalState::Completed)
    );

    rig.prove();
    rig.frame();
    assert_eq!(
        rig.status(),
        RunnerStatus::Passed,
        "a running script and a completed script both leave the PASS"
    );
}

#[test]
fn a_failed_receipt_of_an_earlier_run_does_not_veto_the_pass() {
    let Some(mut rig) = Rig::with(with_peer(with_stat_proof(queue_scenario()))) else {
        return;
    };
    rig.start_on(PEER, blocked("the earlier peer run blocked"));
    rig.slot_tick_on(PEER);
    assert_eq!(
        rig.receipt_state_of(PEER),
        Some(script::ScriptTerminalState::Failed)
    );

    // The companion's next Start is accepted but its run is not installed yet:
    // the slot still holds the failed receipt of the earlier run.
    rig.start(ScriptFlow::Continue);
    rig.frames(3);
    assert_eq!(rig.status(), ON_THE_FINAL_PROOF);
    assert_eq!(
        rig.receipt_state_of(PEER),
        Some(script::ScriptTerminalState::Failed)
    );
    rig.prove();
    rig.frame();
    assert_eq!(
        rig.status(),
        RunnerStatus::Passed,
        "the failed receipt belongs to an earlier run of the companion's slot"
    );
}
