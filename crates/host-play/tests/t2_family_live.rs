//! Real-content Load witness for one local-read T2 family at a time.
//! The caller supplies a script and the exact JSON row it must log. This uses
//! production Play and selected content, not a second gameplay runtime.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use host_play::{mint_live_entries, mint_live_names, run_with_template, ProfileOptions};
use serde_json::{json, Value};
use vault::{Profile, ProfileSettings};

fn required(name: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{name} is required"))
}

#[test]
#[ignore = "requires LIVE=1, isolated HOME/cache, a T2 probe, and Engine A"]
fn local_family_reads_real_content() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let evidence = required("BOT_EVIDENCE_DIR");
    let home = required("HOME");
    assert!(
        home.starts_with(&evidence),
        "HOME must be disposable evidence"
    );
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX").expect("unique live prefix");
    assert!(!prefix.is_empty() && prefix.len() <= 4);
    let source_path = required("BOT_T2_LIVE_SOURCE");
    let expected_path = required("BOT_T2_LIVE_EXPECT");
    let source = std::fs::read_to_string(&source_path).expect("family probe source");
    let source_sha = script::JsCache::origin_sha(source.as_bytes());
    let expected: Value = serde_json::from_slice(
        &std::fs::read(&expected_path).expect("exact expected family result"),
    )
    .expect("expected result is JSON");
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(44594),
        http_port: Some(1080),
        cache_dir: Some(required("BOT_CACHE_DIR")),
        unpack_dir: Some(home.join(".274bot/unpack-289")),
        vault_path: Some(home.join("vault")),
        nav_pack: Some(required("WORLD_NAV_PACK")),
        engine_dir: Some(required("WORLD_ENGINE_DIR")),
        ..ProfileOptions::default()
    };
    let template = options
        .resolve(None)
        .expect("local Engine A profile")
        .prepare_template()
        .expect("real selected-content template");
    let entries = mint_live_entries(&mint_live_names(1));
    let (account, password) = &entries[0];
    let ingame = Arc::new(AtomicBool::new(false));
    let frame_ingame = Arc::clone(&ingame);
    let frame_account = account.clone();
    let mut play = run_with_template(
        template,
        false,
        vec![],
        |_| (None, None),
        move |client, name, input| {
            if name == frame_account {
                frame_ingame.store(
                    !input.hold && client.ingame && client.scene_state == 2,
                    Ordering::Release,
                );
            }
        },
    )
    .expect("start production Play");
    play.try_spawn_slot(
        Profile {
            username: account.clone(),
            password: password.clone().into(),
            uid: 274_289_207,
            settings: ProfileSettings::default(),
        },
        None,
        None,
        None,
    )
    .expect("spawn fresh Engine A account");
    let deadline = Instant::now() + Duration::from_secs(240);
    while !ingame.load(Ordering::Acquire) {
        assert!(
            Instant::now() < deadline,
            "Engine A did not reach a ready scene"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    play.script_start_load(
        account,
        source,
        script::LoadShape::CompatClass,
        None,
        vec![],
    )
    .expect("start real family Load probe");
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut logs = Vec::new();
    let observed = loop {
        logs.extend(play.script_take_pending_logs(account));
        let results = logs
            .iter()
            .filter_map(|line| line.strip_prefix("T2 family: "))
            .collect::<Vec<_>>();
        assert!(
            results.len() <= 1,
            "family probe logged more than once: {logs:?}"
        );
        if let Some(result) = results.first() {
            break serde_json::from_str::<Value>(result).expect("family result is JSON");
        }
        assert_ne!(
            play.script_state(account),
            script::RunState::Error,
            "family probe failed: {:?}; logs={logs:?}",
            play.script_last_error(account),
        );
        assert!(Instant::now() < deadline, "family result timeout: {logs:?}");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(observed, expected, "real-content family result");
    play.script_stop(account);
    let deadline = Instant::now() + Duration::from_secs(30);
    while play.script_state(account) != script::RunState::Idle {
        assert!(Instant::now() < deadline, "family Load did not stop");
        std::thread::sleep(Duration::from_millis(50));
    }
    logs.extend(play.script_take_pending_logs(account));
    assert_eq!(
        logs.iter()
            .filter(|line| line.starts_with("T2 family: "))
            .count(),
        1,
        "one completed family probe",
    );
    let result = json!({
        "verdict": "PASS",
        "account": account,
        "source": source_path,
        "source_sha256": source_sha,
        "expected_source": expected_path,
        "result": observed,
        "game_port": 44594,
        "http_port": 1080,
        "stopped": true,
    });
    let result_path = evidence.join("family-live-result.json");
    std::fs::write(&result_path, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    println!("family live result: {}\n{result}", result_path.display());
    drop(play);
}
