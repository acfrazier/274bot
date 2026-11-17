//! Producer-thread allocation and retained-size gates for selected actors.
use api::snapshot::{actor_observation, ActorObservationView, ActorSelector};
use client::client::{Client, ClientConfig};
use client::dash3d::ClientPlayer;
use scenario::actor::ActorObserver;
use scenario::{
    Proof, RunnerStatus, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait,
};

fn client() -> Client {
    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "absent-actor-allocation-cache".into(),
        members: true,
        lowmem: false,
    });
    client.ingame = true;
    client.scene_state = 2;
    client.self_slot = 0;
    let mut player = ClientPlayer::at(20, 20);
    player.name = Some("Alice".into());
    player.chat_message = Some("Hello".into());
    player.chat_timer = 100;
    client.local_player = Some(player);
    client.map_build_base_x = 3200;
    client.map_build_base_z = 3200;
    client.bump_gens(client::io::ServerProt::PLAYER_INFO);
    client
}

fn runner(armed: bool) -> ScenarioRunner {
    let proof = if armed {
        Proof::OverheadText {
            actor: ActorSelector::LocalPlayer,
            text: "never",
        }
    } else {
        Proof::StatAtMost { id: 3, max: -1 }
    };
    let mut runner = ScenarioRunner::with_world(
        Scenario {
            name: "actor-allocation",
            seed: Seed {
                profiles: vec![("test", "test")],
                mainland: false,
            },
            steps: vec![Step {
                name: "fixed-input observation",
                kind: if armed {
                    StepKind::ObserveActor {
                        actor: ActorSelector::LocalPlayer,
                    }
                } else {
                    StepKind::Shot { label: "unarmed" }
                },
                wait: Wait {
                    arm: proof,
                    budget_ticks: u32::MAX,
                },
            }],
            proof,
            companions: vec![],
            settings: ScenarioSettings::default(),
        },
        None,
    );
    runner.set_scene_settle(std::time::Duration::ZERO);
    runner
}

fn assert_chat_records(evidence: &scenario::actor::ActorEvidence) {
    use scenario::actor::ActorRecord;
    assert!(evidence.records.iter().any(|record| matches!(
        record,
        ActorRecord::ActorSample { name: Some(name), .. } if name == "Alice"
    )));
    assert!(evidence
        .records
        .iter()
        .any(|record| matches!(record, ActorRecord::ActorStyle { .. })));
    assert!(evidence.records.iter().any(|record| matches!(
        record,
        ActorRecord::Overhead { text: Some(text), transition: "appeared", .. } if text == "Hello"
    )));
    assert!(!evidence
        .records
        .iter()
        .any(|record| matches!(record, ActorRecord::Unavailable { .. })));
}

#[test]
fn actor_scalar_and_event_observation_allocate_nothing_disarmed_or_unchanged_armed() {
    std::thread::spawn(|| {
        let mut client = client();
        for _ in 0..8 {
            assert!(actor_observation(&client, Some(ActorSelector::LocalPlayer)).is_some());
            client.local_player.as_mut().unwrap().chat_message = None;
            let mut owner = Some(Box::<ActorObserver>::default());
            ActorObserver::observe(
                owner.as_deref_mut(),
                &client,
                Some(ActorSelector::LocalPlayer),
            )
            .unwrap();
            client.loop_cycle += 1;
            client.local_player.as_mut().unwrap().chat_message = Some("Hello".into());
            ActorObserver::observe(
                owner.as_deref_mut(),
                &client,
                Some(ActorSelector::LocalPlayer),
            )
            .unwrap();
            let armed = allocation_counter::measure(|| {
                for _ in 0..1000 {
                    std::hint::black_box(actor_observation(
                        &client,
                        Some(ActorSelector::LocalPlayer),
                    ));
                    ActorObserver::observe(
                        std::hint::black_box(owner.as_deref_mut()),
                        &client,
                        Some(ActorSelector::LocalPlayer),
                    )
                    .unwrap();
                }
            });
            assert_eq!(armed.count_total, 0, "armed: {armed:?}");
            assert_eq!(armed.bytes_total, 0, "armed: {armed:?}");
            ActorObserver::observe(owner.as_deref_mut(), &client, None).unwrap();
            let disarmed_owner = allocation_counter::measure(|| {
                for cycle in 0..1000 {
                    client.loop_cycle += 1;
                    client.local_player.as_mut().unwrap().chat_timer = 100 - cycle % 100;
                    ActorObserver::observe(
                        std::hint::black_box(owner.as_deref_mut()),
                        &client,
                        None,
                    )
                    .unwrap();
                }
            });
            assert_eq!(disarmed_owner.count_total, 0, "owner: {disarmed_owner:?}");
            assert_eq!(disarmed_owner.bytes_total, 0, "owner: {disarmed_owner:?}");
            let evidence = owner.take().unwrap().into_evidence();
            assert_chat_records(&evidence);
            assert_eq!(evidence.records.len(), 3, "disarmed owner recorded changes");
            client.gens.session += 1;
            client.local_player.as_mut().unwrap().chat_timer = 100;
            let disarmed = allocation_counter::measure(|| {
                for _ in 0..1000 {
                    std::hint::black_box(actor_observation(&client, None));
                    ActorObserver::observe(
                        std::hint::black_box(owner.as_deref_mut()),
                        &client,
                        Some(ActorSelector::LocalPlayer),
                    )
                    .unwrap();
                }
            });
            assert_eq!(disarmed.count_total, 0, "disarmed: {disarmed:?}");
            assert_eq!(disarmed.bytes_total, 0, "disarmed: {disarmed:?}");
        }
    })
    .join()
    .expect("actor sampler producer thread");
}

#[test]
fn runner_actor_observation_allocates_nothing_disarmed_or_unchanged_armed() {
    std::thread::spawn(|| {
        let mut client = client();
        for armed in [false, true] {
            for _ in 0..8 {
                let mut runner = runner(armed);
                client.local_player.as_mut().unwrap().chat_message = None;
                for _ in 0..8 {
                    runner.tick(&mut client);
                }
                client.loop_cycle += 1;
                client.local_player.as_mut().unwrap().chat_message = Some("Hello".into());
                runner.tick(&mut client);
                assert!(matches!(runner.status(), RunnerStatus::Running { .. }));
                let counts = allocation_counter::measure(|| {
                    for cycle in 0..1000 {
                        if !armed {
                            client.loop_cycle += 1;
                            client.local_player.as_mut().unwrap().chat_timer = 100 - cycle % 100;
                        }
                        runner.tick(std::hint::black_box(&mut client));
                    }
                });
                assert_eq!(counts.count_total, 0, "runner armed={armed}: {counts:?}");
                assert_eq!(counts.bytes_total, 0, "runner armed={armed}: {counts:?}");
                assert!(matches!(runner.status(), RunnerStatus::Running { .. }));
                runner.fail_start("allocation measurement finished");
                assert_eq!(runner.evidence().unwrap().actors.is_some(), armed);
                if let Some(actors) = &runner.evidence().unwrap().actors {
                    assert_chat_records(actors);
                }
                client.local_player.as_mut().unwrap().chat_timer = 100;
                client.gens.session += 1;
            }
        }
    })
    .join()
    .expect("actor runner producer thread");
}

#[test]
fn selected_actor_owner_retains_bounded_scalar_state() {
    println!(
        "ActorObservationView={} ActorObserver={} ScenarioRunner={}",
        std::mem::size_of::<ActorObservationView<'_>>(),
        std::mem::size_of::<ActorObserver>(),
        std::mem::size_of::<ScenarioRunner>()
    );
    // Measured optional owner / borrowed view / observer: 8 / 136 / 136 B + 64 B.
    assert!(std::mem::size_of::<Option<Box<ActorObserver>>>() <= 72);
    assert!(std::mem::size_of::<ActorObservationView<'_>>() <= 200);
    assert!(std::mem::size_of::<ActorObserver>() <= 200);
}
