//! A state-only Engine A receipt cell. It makes no declared-cadence assertion.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use api::snapshot::GameSnapshot;
use api::tick_health::TickHealthReceipt;
use vault::{Profile, ProfileSettings};

#[derive(Default)]
struct Observations {
    pump: host::Pump,
    snapshot: GameSnapshot,
    receipts: Vec<TickHealthReceipt>,
}

#[test]
#[ignore = "requires LIVE=1 and a local revision-289 engine"]
fn tick_health_engine_a_receipt() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let root =
        std::path::PathBuf::from(std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR"));
    let scratch = root.join("tick-health-cell");
    std::fs::create_dir_all(&scratch).unwrap();
    let profile = super::bank_core_live::live_profile(&scratch)
        .unwrap()
        .resolve(None)
        .and_then(|profile| profile.bind_runtime())
        .unwrap();
    assert_eq!(profile.client().game_host(), "127.0.0.1");
    assert_eq!(profile.client().game_port(), 44594);
    let account = super::mint_live_names(1).pop().unwrap();
    let template = super::SharedClientTemplate::load(profile).unwrap();
    let observations = Arc::new(Mutex::new(Observations::default()));
    let frame_observations = Arc::clone(&observations);
    let mut play = super::run_with_template(
        template,
        true,
        vec![Profile {
            username: account.clone(),
            password: account.clone().into(),
            uid: 274_279_112,
            settings: ProfileSettings::default(),
        }],
        |_| (None, None),
        move |client, _, _| {
            let mut observations = frame_observations.lock().unwrap();
            let drain = observations.pump.drain_client(client);
            host::publish_snapshot(&mut observations.snapshot, client, drain);
            if observations.snapshot.ingame()
                && observations.snapshot.scene_state() == 2
                && drain.packet_delta != 0
                && observations.receipts.len() < 16
            {
                let receipt = observations.snapshot.tick_health();
                assert_eq!(receipt.packet_delta, drain.packet_delta);
                observations.receipts.push(receipt);
            }
        },
    )
    .unwrap();
    let started = Instant::now();
    while observations.lock().unwrap().receipts.len() < 8
        && started.elapsed() < Duration::from_secs(120)
    {
        std::thread::sleep(Duration::from_millis(50));
    }
    play.stop_slot(&account);
    let observations = observations.lock().unwrap();
    let receipts = &observations.receipts;
    let complete = receipts.len() >= 8;
    let receipt = serde_json::json!({
        "verdict": if complete { "PASS" } else { "FAIL" },
        "source_sha": std::env::var("LIVE_SOURCE_SHA").ok(),
        "account": account,
        "source": "applied PLAYER_INFO generation at monotonic drain receive observations",
        "host_service_delay_scope": "previous completed service, one-service lag",
        "elapsed_ms": started.elapsed().as_millis(),
        "receipts": receipts,
    });
    std::fs::write(
        root.join("tick-health-receipt.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
    assert!(
        complete,
        "cell wall watchdog ended before eight packet drains"
    );
    for pair in receipts.windows(2) {
        if pair[0].session == pair[1].session {
            assert_eq!(
                pair[1]
                    .packet_generation
                    .wrapping_sub(pair[0].packet_generation),
                pair[1].packet_delta,
            );
            assert_eq!(
                pair[1].server_ticks - pair[0].server_ticks,
                pair[1].packet_delta,
            );
        }
    }
}
