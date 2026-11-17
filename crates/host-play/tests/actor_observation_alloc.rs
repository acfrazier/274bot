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

#[test]
fn actor_scalar_and_event_observation_allocate_nothing_disarmed_or_unchanged_armed() {
    std::thread::spawn(|| {
        let mut client = client();
        for _ in 0..8 {
            let mut owner = Some(Box::<ActorObserver>::default());
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
            drop(owner.take());
            client.gens.session += 1;
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
                for _ in 0..8 {
                    runner.tick(&mut client);
                }
                assert!(matches!(runner.status(), RunnerStatus::Running { .. }));
                let counts = allocation_counter::measure(|| {
                    for _ in 0..1000 {
                        runner.tick(std::hint::black_box(&mut client));
                    }
                });
                assert_eq!(counts.count_total, 0, "runner armed={armed}: {counts:?}");
                assert_eq!(counts.bytes_total, 0, "runner armed={armed}: {counts:?}");
                assert!(matches!(runner.status(), RunnerStatus::Running { .. }));
                runner.fail_start("allocation measurement finished");
                assert_eq!(runner.evidence().unwrap().actors.is_some(), armed);
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
    // Generous ceilings leave room for ordinary layout changes, not exact-size locks.
    assert!(std::mem::size_of::<Option<Box<ActorObserver>>>() <= 72);
    assert!(std::mem::size_of::<ActorObservationView<'_>>() <= 320);
    assert!(std::mem::size_of::<ActorObserver>() <= 512);
    assert!(std::mem::size_of::<ScenarioRunner>() <= 6400);
}
