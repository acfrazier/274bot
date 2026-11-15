use std::time::Duration;

use client::client::{Client, ClientConfig};
use client::dash3d::ClientPlayer;
use client::io::ServerProt;
use host_play::evidence_writer::{EvidenceRequest, EvidenceWriter, PngColor};
use scenario::{Proof, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait};
use sha2::{Digest, Sha256};

#[test]
fn terminal_manifest_accepts_durable_writer_artifact() {
    let mut runner = ScenarioRunner::with_world(
        Scenario {
            name: "terminal-artifact-manifest",
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
        },
        None,
    );
    runner.set_scene_settle(Duration::ZERO);
    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: std::env::temp_dir()
            .join("absent-artifact-cache")
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
        runner.tick(&mut client);
    }
    assert_eq!(runner.evidence().unwrap().outcome, "PASS");
    let directory =
        std::env::temp_dir().join(format!("274bot-terminal-manifest-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("terminal.png");
    let writer = EvidenceWriter::new(1);
    let job = writer.submit(EvidenceRequest {
        png_path: path.clone(),
        width: 2,
        height: 2,
        pixels: vec![0x12_3456; 4],
        color: PngColor::Rgba,
        sidecar: None,
    });
    assert!(writer.flush(Duration::from_secs(30)));
    assert!(writer.poll(&job).unwrap().error.is_none());
    let bytes = std::fs::read(&path).unwrap();
    let expected = serde_json::json!({
        "path": "terminal.png",
        "bytes": bytes.len(),
        "sha256": format!("{:x}", Sha256::digest(&bytes)),
    });
    if let Some(manifest) = runner.receipt_manifest_mut() {
        manifest.artifacts.push(expected.clone());
    }
    let receipt: serde_json::Value =
        serde_json::from_str(&runner.evidence().unwrap().to_json()).unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    assert_eq!(
        receipt["manifest"]["artifacts"],
        serde_json::json!([expected])
    );
}
