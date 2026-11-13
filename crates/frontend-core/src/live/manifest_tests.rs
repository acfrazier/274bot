use std::time::Duration;

use client::client::{Client, ClientConfig};
use client::dash3d::ClientPlayer;
use client::io::ServerProt;
use scenario::{Proof, ScenarioSettings, Seed, Step, StepKind, Wait};

#[test]
fn live_driver_terminal_receipt_contains_source_identities() {
    let session = crate::OperatorSession::<()>::open(crate::launch::InstancePermit::skip());
    let scenario = scenario::Scenario {
        name: "driver-source-manifest",
        seed: Seed {
            profiles: vec![("alice", "alice")],
            mainland: false,
        },
        steps: vec![Step {
            name: "observe the ready client",
            kind: StepKind::Await {
                evidence: "ready",
                ready: Box::new(|_| true),
                summary: None,
            },
            wait: Wait {
                arm: Proof::IngameScene2,
                budget_ticks: 10,
            },
        }],
        proof: Proof::IngameScene2,
        companions: vec![],
        settings: ScenarioSettings::default(),
    };
    let mut runner =
        session.live_scenario_runner(scenario.into(), &["alice".into()], "127.0.0.1", 43594, None);
    runner.0.set_scene_settle(Duration::ZERO);
    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: std::env::temp_dir()
            .join("absent-manifest-cache")
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
    for _ in 0..4 {
        client.bump_gens(ServerProt::PLAYER_INFO);
        runner.0.tick(&mut client);
    }
    let receipt: serde_json::Value =
        serde_json::from_str(&runner.0.evidence().expect("terminal receipt").to_json()).unwrap();
    assert_eq!(receipt["outcome"], "PASS");
    let sources = &receipt["manifest"]["sources"];
    for name in ["host", "client"] {
        assert!(
            sources[name]["commit"]
                .as_str()
                .is_some_and(|value| !value.is_empty()),
            "missing {name} build identity: {sources}"
        );
        assert!(sources[name]["worktree_dirty"].is_boolean(), "{sources}");
    }
}
