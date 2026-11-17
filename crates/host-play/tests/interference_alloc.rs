//! Fixed-input observer proof, including the real producer thread and reallocation.
use api::snapshot::GameSnapshot;
use scenario::interference::{CombatObserver, CombatSample};
use scenario::{
    CombatExpectation, Proof, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind,
    Wait,
};

#[test]
fn disarmed_combat_observation_allocates_nothing_after_repeated_owner_and_session_changes() {
    std::thread::spawn(|| {
        let snapshot = GameSnapshot::new();
        for session in 0..8 {
            let mut owner = Some(Box::<CombatObserver>::default());
            CombatObserver::observe(owner.as_deref_mut(), &snapshot, session);
            drop(owner.take());
            CombatObserver::observe(owner.as_deref_mut(), &snapshot, session + 1);
            let counts = allocation_counter::measure(|| {
                for _ in 0..1000 {
                    CombatObserver::observe(
                        std::hint::black_box(owner.as_deref_mut()),
                        &snapshot,
                        session + 1,
                    );
                }
            });
            assert_eq!(
                counts.count_total, 0,
                "disarmed combat observer allocated or reallocated: {counts:?}"
            );
            assert_eq!(
                counts.bytes_total, 0,
                "disarmed combat observer retained bytes"
            );
        }
    })
    .join()
    .expect("observer producer thread");
}

#[test]
fn unchanged_armed_window_polls_allocate_nothing_on_the_producer_thread() {
    std::thread::spawn(|| {
        let mut client = client::client::Client::new(client::client::ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: std::env::temp_dir()
                .join("absent-combat-cache")
                .to_string_lossy()
                .into_owned(),
            members: true,
            lowmem: false,
        });
        client.ingame = true;
        client.scene_state = 2;
        client.local_player = Some(client::dash3d::ClientPlayer::at(20, 20));
        client.stat_effective_level[3] = 10;
        client.map_build_base_x = 3200;
        client.map_build_base_z = 3200;
        for prot in [
            client::io::ServerProt::PLAYER_INFO,
            client::io::ServerProt::REBUILD_NORMAL,
            client::io::ServerProt::UPDATE_STAT,
        ] {
            client.bump_gens(prot);
        }
        for _ in 0..8 {
            let mut runner = ScenarioRunner::with_world(
                Scenario {
                    name: "armed-observer-allocation",
                    seed: Seed {
                        profiles: vec![("test", "test")],
                        mainland: false,
                    },
                    steps: vec![
                        Step {
                            name: "Begin",
                            kind: StepKind::BeginMeasuredWindow {
                                name: "quiet",
                                expectation: CombatExpectation::Quiet,
                            },
                            wait: Wait {
                                arm: Proof::IngameScene2,
                                budget_ticks: 2,
                            },
                        },
                        Step {
                            name: "unchanged armed observation",
                            kind: StepKind::Shot { label: "armed" },
                            wait: Wait {
                                arm: Proof::StatAtMost { id: 3, max: 0 },
                                budget_ticks: u32::MAX,
                            },
                        },
                        Step {
                            name: "End",
                            kind: StepKind::EndMeasuredWindow { name: "quiet" },
                            wait: Wait {
                                arm: Proof::IngameScene2,
                                budget_ticks: 2,
                            },
                        },
                    ],
                    proof: Proof::IngameScene2,
                    companions: vec![],
                    settings: ScenarioSettings::default(),
                },
                None,
            );
            runner.set_scene_settle(std::time::Duration::ZERO);
            for _ in 0..8 {
                runner.tick(&mut client);
            }
            assert!(matches!(
                runner.status(),
                scenario::RunnerStatus::Running { .. }
            ));
            // Construction, first snapshot, bounded window/event growth and NPC
            // disclosure may allocate. Unchanged armed samples may not.
            let counts = allocation_counter::measure(|| {
                for _ in 0..1000 {
                    runner.tick(std::hint::black_box(&mut client));
                }
            });
            assert_eq!(
                counts.count_total, 0,
                "armed poll allocated/reallocated: {counts:?}"
            );
            assert_eq!(
                counts.bytes_total, 0,
                "armed poll allocated bytes: {counts:?}"
            );
            assert!(matches!(
                runner.status(),
                scenario::RunnerStatus::Running { .. }
            ));
            runner.fail_start("finish the allocation measurement");
            let combat = runner.evidence().unwrap().interference.as_ref().unwrap();
            assert_eq!(
                combat.windows.len(),
                1,
                "the measured observer must have been armed"
            );
            assert_eq!(combat.windows[0].end_reason, Some("terminal"));
            client.gens.session += 1;
        }
    })
    .join()
    .expect("armed observer producer thread");
}

#[test]
fn measured_combat_owner_has_fixed_state_size() {
    println!(
        "CombatSample={} CombatObserver={} ScenarioRunner={}",
        std::mem::size_of::<CombatSample>(),
        std::mem::size_of::<CombatObserver>(),
        std::mem::size_of::<scenario::ScenarioRunner>()
    );
    // Measured 8 / 128 / 216 / 5,832 B; each ceiling leaves 64 B.
    assert!(std::mem::size_of::<Option<Box<CombatObserver>>>() <= 72);
    assert!(std::mem::size_of::<CombatSample>() <= 192);
    assert!(std::mem::size_of::<CombatObserver>() <= 280);
    assert!(std::mem::size_of::<scenario::ScenarioRunner>() <= 5_896);
}
