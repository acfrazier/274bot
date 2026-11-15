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
const WATCH: &str = "watch the queued Imp Catcher completion";
const NATIVE_REASON: &str = "search_result=Refusal; NoWayOut: ends inside Lesser demon (L82); resolve the condition, then start the Quester again";
/// The runner waits on its final watch, the second of two steps.
const ON_THE_FINAL_WATCH: RunnerStatus = RunnerStatus::Running { step: 1, total: 2 };

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

/// A live runner whose staged Start is already accepted and settled, over a
/// play with no connection and a synthetic client.
struct Rig {
    handle: RunnerHandle,
    play: host_play::Play,
    client: Client,
    slot_tick: u64,
}

impl Rig {
    /// `None` where the generated game data the native test script needs is
    /// absent (hosted CI).
    fn new() -> Option<Self> {
        if !api::test_support::game_data_available() {
            return None;
        }
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
        let mut runner = ScenarioRunner::with_world(queue_scenario(), None);
        runner.set_scene_settle(Duration::ZERO);
        let handle = RunnerHandle::default();
        handle.install_scenario_runner_for_test(runner);
        handle.install_play(&play);
        let mut card = host_play::live_start::PendingCatalogStart::compiled(
            SLOT,
            script::CompiledId("Quester"),
            serde_json::Map::new(),
        );
        card.started = true;
        card.settled = true;
        handle.stage(StartPlan(vec![card]));
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
        Some(Self {
            handle,
            play,
            client,
            slot_tick: 0,
        })
    }

    /// Start a native test script on the slot, as the pump's accepted Start does.
    fn start(&self, flow: ScriptFlow) -> api::selected::RunKey {
        self.play
            .script_start_handle()
            .start_test_script(SLOT, Box::new(EndsRun(flow)), None)
            .expect("the test script starts")
    }

    /// One client frame on the driven slot, with one observed game tick.
    fn frame(&mut self) {
        self.client.bump_gens(ServerProt::PLAYER_INFO);
        self.handle.frame(
            &mut self.client,
            SLOT,
            false,
            ScenarioRunner::tick_with_hold,
            ScenarioRunner::companion_tick,
        );
    }

    fn frames(&mut self, count: usize) {
        for _ in 0..count {
            self.frame();
        }
    }

    /// One observed game tick on the slot: the script runs its tick.
    fn slot_tick(&mut self) {
        self.slot_tick += 1;
        self.play
            .script_start_handle()
            .tick_test_script(SLOT, self.slot_tick, &mut self.client);
    }

    fn status(&self) -> RunnerStatus {
        self.handle.status().expect("an installed runner")
    }

    fn receipt_state(&self) -> Option<script::ScriptTerminalState> {
        self.play
            .script_lifecycle_receipt(SLOT)
            .map(|receipt| receipt.state)
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
    let message = rig.handle.failure_message().expect("the failure message");
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
    let message = rig.handle.failure_message().expect("the failure message");
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
