//! Runs in the workspace CI lane, with or without host-play default features.
//! The counter includes allocations and reallocations on the producer thread.
use std::time::Duration;

use client::client::{Client, ClientConfig};
use client::dash3d::ClientPlayer;
use scenario::{Proof, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait};

#[test]
fn disarmed_receipts_allocate_nothing_on_tick_and_paint_producer() {
    std::thread::spawn(|| {
        static QUANTITY_PROOFS: [Proof; 3] = [
            Proof::ProductDelta { id: 52, count: 30 },
            Proof::XpDelta { id: 9, min: 10 },
            Proof::InputConsumed { id: 1511, count: 2 },
        ];
        let scenario = Scenario {
            name: "receipt-allocation-unarmed-quantity",
            seed: Seed {
                profiles: vec![("test", "test")],
                mainland: false,
            },
            steps: vec![Step {
                name: "observe without sending a quantity action",
                kind: StepKind::Shot { label: "unarmed" },
                wait: Wait {
                    arm: Proof::IngameScene2,
                    budget_ticks: u32::MAX,
                },
            }],
            proof: Proof::All(&QUANTITY_PROOFS),
            companions: vec![],
            settings: ScenarioSettings::default(),
        };
        let mut runner = ScenarioRunner::with_world(scenario, None);
        runner.set_scene_settle(Duration::ZERO);
        let mut client = Client::new(ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: std::env::temp_dir()
                .join("absent-receipt-cache")
                .to_string_lossy()
                .into_owned(),
            members: true,
            lowmem: false,
        });
        client.ingame = true;
        client.scene_state = 2;
        client.map_build_base_x = 3200;
        client.map_build_base_z = 3200;
        client.local_player = Some(ClientPlayer::at(20, 20));
        for _ in 0..8 {
            runner.set_receipt_recording(true);
            runner.tick(&mut client);
            runner.set_receipt_recording(false);
            client.gens.session += 1;
            runner.tick(&mut client);
            runner.observe_script_paint(["unrequested paint"]);
            // Warm the ordinary snapshot after replacing the session. Fixed
            // inputs isolate the disarmed observer from snapshot allocations.
            let counts = allocation_counter::measure(|| {
                for _ in 0..1000 {
                    runner.tick(&mut client);
                    runner.observe_script_paint(["unrequested paint"]);
                    assert!(runner.receipt_manifest_mut().is_none());
                }
            });
            assert_eq!(
                counts.count_total, 0,
                "disarmed quantity observer allocated: {counts:?}"
            );
            assert_eq!(counts.bytes_total, 0, "disarmed observer retained bytes");
        }
    })
    .join()
    .expect("producer thread");
}

#[test]
fn scenario_runner_quantity_baseline_storage_is_pinned() {
    // The +8 bytes over the earlier 5,640 pin come from merged dev's
    // FollowRun::walk_packet_budget in crates/nav-executor/src/traveller.rs,
    // not this branch's quantity fields.
    assert_eq!(std::mem::size_of::<ScenarioRunner>(), 5_648);
}
