use api::snapshot::{ActorSelector, GameSnapshot};
use client::client::{Client, ClientConfig};
use client::dash3d::ClientPlayer;
use client::io::ServerProt;
use scenario::{
    Proof, RunnerStatus, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait,
};
use std::time::Duration;

const ACTOR: ActorSelector = ActorSelector::LocalPlayer;
const TEXT: &str = "Hello";

fn client() -> Client {
    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "absent-actor-cache".into(),
        members: true,
        lowmem: false,
    });
    client.ingame = true;
    client.scene_state = 2;
    client.self_slot = 0;
    client.local_player = Some(ClientPlayer::at(20, 20));
    client.local_player.as_mut().unwrap().name = Some("Alice".into());
    client.map_build_base_x = 3200;
    client.map_build_base_z = 3200;
    client.bump_gens(ServerProt::PLAYER_INFO);
    client
}

fn scenario(proof: Proof) -> Scenario {
    Scenario {
        name: "actor-observation",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "observe real actor",
            kind: StepKind::ObserveActor { actor: ACTOR },
            wait: Wait {
                arm: proof,
                budget_ticks: 20,
            },
        }],
        proof,
        companions: vec![],
        settings: ScenarioSettings::default(),
    }
}

fn runner(proof: Proof) -> ScenarioRunner {
    let mut runner = ScenarioRunner::with_world(scenario(proof), None);
    runner.set_scene_settle(Duration::ZERO);
    runner
}

fn bubble(client: &mut Client, text: Option<&str>, timer: i32) {
    client.loop_cycle += 1;
    let player = client.local_player.as_mut().unwrap();
    player.chat_message = text.map(str::to_owned);
    player.chat_timer = timer;
    player.chat_colour = 1;
    player.chat_effect = 2;
}

fn prime(runner: &mut ScenarioRunner, client: &mut Client) {
    for _ in 0..4 {
        runner.tick(client);
    }
    assert!(matches!(runner.status(), RunnerStatus::Running { .. }));
}

#[test]
fn actor_style_and_text_require_an_armed_matching_actor_and_never_send() {
    for proof in [
        Proof::ActorChatStyle {
            actor: ACTOR,
            colour: 1,
            effect: 2,
        },
        Proof::OverheadText {
            actor: ACTOR,
            text: TEXT,
        },
    ] {
        assert!(!proof.check(&GameSnapshot::new(), None));
        let mut runner = runner(proof);
        let mut client = client();
        prime(&mut runner, &mut client);
        bubble(&mut client, Some("Wrong"), 100);
        client.local_player.as_mut().unwrap().chat_colour = 7;
        runner.tick(&mut client);
        assert!(matches!(runner.status(), RunnerStatus::Running { .. }));
        bubble(&mut client, Some(TEXT), 100);
        runner.tick(&mut client);
        assert_eq!(runner.status(), RunnerStatus::Passed);
        assert_eq!(client.out.pos, 0);
        let receipt = runner.evidence().unwrap();
        assert!(!receipt.actors.as_ref().unwrap().records.is_empty());
        assert!(receipt.to_json().contains("\"actor_style\""));
    }
}

#[test]
fn overhead_lifetime_requires_appearance_countdown_and_natural_expiry() {
    let proof = Proof::OverheadTextLifetime {
        actor: ACTOR,
        text: TEXT,
    };
    let mut runner = runner(proof);
    let mut client = client();
    prime(&mut runner, &mut client);
    bubble(&mut client, Some(TEXT), 3);
    runner.tick(&mut client);
    bubble(&mut client, Some(TEXT), 2);
    runner.tick(&mut client);
    assert!(matches!(runner.status(), RunnerStatus::Running { .. }));
    bubble(&mut client, None, 0);
    runner.tick(&mut client);
    assert_eq!(runner.status(), RunnerStatus::Passed);
    assert_eq!(client.out.pos, 0);
    let receipt = runner.evidence().unwrap();
    let records = &receipt.actors.as_ref().unwrap().records;
    assert!(records.iter().any(|record| matches!(
        record,
        scenario::actor::ActorRecord::Overhead {
            transition: "appeared",
            ..
        }
    )));
    assert!(records.iter().any(|record| matches!(
        record,
        scenario::actor::ActorRecord::Overhead {
            transition: "countdown",
            ..
        }
    )));
    assert!(records.iter().any(|record| matches!(
        record,
        scenario::actor::ActorRecord::Overhead {
            transition: "expired",
            ..
        }
    )));
}

#[test]
fn disappearance_replacement_session_change_and_disarm_cannot_fake_expiry() {
    for cause in [
        "missing",
        "replacement",
        "session",
        "clear",
        "identity",
        "rearmed",
    ] {
        let mut observer = scenario::actor::ActorObserver::default();
        let mut client = client();
        for timer in [3, 2] {
            bubble(&mut client, Some(TEXT), timer);
            scenario::actor::ActorObserver::observe(Some(&mut observer), &client, Some(ACTOR))
                .unwrap();
        }
        match cause {
            "missing" => {
                client.scene_state = 1;
                scenario::actor::ActorObserver::observe(Some(&mut observer), &client, Some(ACTOR))
                    .unwrap();
                client.scene_state = 2;
            }
            "replacement" => {
                bubble(&mut client, Some("Other"), 100);
                scenario::actor::ActorObserver::observe(Some(&mut observer), &client, Some(ACTOR))
                    .unwrap();
            }
            "session" => client.gens.session += 1,
            "clear" => {}
            "identity" => client.local_player.as_mut().unwrap().name = Some("Bob".into()),
            "rearmed" => {
                scenario::actor::ActorObserver::observe(Some(&mut observer), &client, None).unwrap()
            }
            _ => unreachable!(),
        }
        bubble(&mut client, None, if cause == "clear" { 1 } else { 0 });
        scenario::actor::ActorObserver::observe(Some(&mut observer), &client, Some(ACTOR)).unwrap();
        let evidence = observer.into_evidence();
        assert!(
            !evidence.records.iter().any(|record| matches!(
                record,
                scenario::actor::ActorRecord::Overhead {
                    transition: "expired",
                    message_record: Some(2),
                    ..
                }
            )),
            "{cause}"
        );
    }
}

#[test]
fn malformed_actor_proofs_are_refused_before_dispatch_and_cancel_drains_records() {
    let proof = Proof::OverheadTextLifetime {
        actor: ACTOR,
        text: TEXT,
    };
    for case in ["unpaired", "wrong-actor", "nested", "terminal"] {
        let mut scenario = scenario(proof);
        match case {
            "unpaired" => scenario.steps[0].kind = StepKind::Shot { label: "unarmed" },
            "wrong-actor" => {
                scenario.steps[0].kind = StepKind::ObserveActor {
                    actor: ActorSelector::Player(7),
                }
            }
            "nested" => {
                scenario.steps[0].wait.arm = Proof::All(&[Proof::OverheadText {
                    actor: ACTOR,
                    text: TEXT,
                }])
            }
            "terminal" => {
                scenario.proof = Proof::OverheadText {
                    actor: ActorSelector::Player(7),
                    text: TEXT,
                }
            }
            _ => unreachable!(),
        }
        let mut runner = ScenarioRunner::with_world(scenario, None);
        let mut client = client();
        runner.tick(&mut client);
        assert!(matches!(runner.status(), RunnerStatus::Failed(_)), "{case}");
        assert_eq!(client.out.pos, 0);
        assert!(runner.evidence().unwrap().actors.is_none());
    }
    let mut runner = runner(proof);
    let mut client = client();
    prime(&mut runner, &mut client);
    runner.fail_start("cancelled actor watch");
    let before = runner.evidence().unwrap().to_json();
    for _ in 0..3 {
        runner.tick(&mut client);
    }
    assert_eq!(runner.evidence().unwrap().to_json(), before);
    assert!(before.contains("cancelled actor watch"));
}

#[test]
fn actor_record_overflow_is_explicit_and_never_a_pass() {
    let mut runner = runner(Proof::OverheadText {
        actor: ACTOR,
        text: "never",
    });
    let mut client = client();
    prime(&mut runner, &mut client);
    for cycle in 0..1100 {
        bubble(
            &mut client,
            Some(if cycle % 2 == 0 { "One" } else { "Two" }),
            100,
        );
        runner.tick(&mut client);
        if matches!(runner.status(), RunnerStatus::Failed(_)) {
            break;
        }
    }
    let evidence = runner.evidence().unwrap();
    assert_eq!(evidence.outcome, "UNAVAILABLE");
    assert!(evidence.actors.as_ref().unwrap().overflowed);
    assert_eq!(evidence.actors.as_ref().unwrap().records.len(), 1024);
    assert_eq!(client.out.pos, 0);
}

#[test]
fn an_overflowed_actor_observer_can_disarm_for_terminal_cleanup() {
    let mut observer = scenario::actor::ActorObserver::default();
    let mut client = client();
    let mut overflowed = false;
    for cycle in 0..1100 {
        bubble(
            &mut client,
            Some(if cycle % 2 == 0 { "One" } else { "Two" }),
            100,
        );
        if scenario::actor::ActorObserver::observe(Some(&mut observer), &client, Some(ACTOR))
            .is_err()
        {
            overflowed = true;
            break;
        }
    }
    assert!(overflowed);
    scenario::actor::ActorObserver::observe(Some(&mut observer), &client, None).unwrap();
    let evidence = observer.into_evidence();
    assert!(evidence.overflowed);
    assert_eq!(evidence.records.len(), 1024);
}
