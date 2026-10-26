//! Real-content Load witness for one local-read T2 family at a time.
//! The caller supplies a script and the exact JSON row it must log. This uses
//! production Play and selected content, not a second gameplay runtime.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use api::interact;
use api::snapshot::GameSnapshot;
use client::client::Client;
use host_play::{mint_live_entries, mint_live_names, run_with_template, ProfileOptions};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use vault::{Profile, ProfileSettings};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct GatherSeed {
    skill: GatherSkill,
    level: u8,
    tool_alias: String,
    tool_id: u32,
    tile: SeedTile,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum GatherSkill {
    Woodcutting,
    Mining,
    Fishing,
}

impl GatherSkill {
    fn name(self) -> &'static str {
        match self {
            Self::Woodcutting => "woodcutting",
            Self::Mining => "mining",
            Self::Fishing => "fishing",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct SeedTile {
    x: i32,
    z: i32,
    level: i32,
}

impl GatherSeed {
    fn parse(bytes: &[u8]) -> Result<Self, String> {
        let seed: Self = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        if !(1..=99).contains(&seed.level) {
            return Err("level must be between 1 and 99".into());
        }
        if seed.tool_alias.is_empty()
            || !seed
                .tool_alias
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err("tool_alias must be one ASCII content-identifier token".into());
        }
        Ok(seed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrepPhase {
    InitialReady,
    InitialTutorial,
    WaitInitialTutorial,
    WaitLogout,
    WaitRelog,
    ReseedTutorial,
    WaitReseed,
    SeedAccount,
    WaitSeed,
    Ready,
}

#[derive(Default)]
struct SeedPrep {
    phase: Option<PrepPhase>,
    error: Option<String>,
    tutorial_reseed: Option<scenario::tutorial::PostRelogTutorial>,
    evidence: Option<Value>,
}

fn send_cheat(client: &mut Client, command: &str) -> Result<(), String> {
    if interact::cheat(client, command).is_sent() {
        Ok(())
    } else {
        Err(format!("fixture command refused: {command}"))
    }
}

fn seed_frame(client: &mut Client, held: bool, seed: &GatherSeed, shared: &Mutex<SeedPrep>) {
    let mut prep = shared.lock().unwrap();
    if prep.error.is_some() || prep.phase == Some(PrepPhase::Ready) {
        return;
    }
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(client);
    if prep.phase == Some(PrepPhase::WaitLogout) && !client.ingame {
        prep.phase = Some(PrepPhase::WaitRelog);
        return;
    }
    if !held {
        let result = (|| -> Result<(), String> {
            match prep.phase.unwrap_or(PrepPhase::InitialReady) {
                PrepPhase::InitialReady if client.ingame && client.scene_state == 2 => {
                    interact::mainland_hop(client);
                    prep.phase = Some(PrepPhase::InitialTutorial);
                }
                PrepPhase::InitialTutorial => {
                    send_cheat(client, scenario::tutorial::TUTORIAL_GETVAR)?;
                    prep.phase = Some(PrepPhase::WaitInitialTutorial);
                }
                PrepPhase::WaitInitialTutorial
                    if scenario::tutorial::tutorial_confirmed(&snapshot, 0) =>
                {
                    let ifaces = Arc::clone(&client.ifaces);
                    if !interact::logout(client, &ifaces) {
                        return Err("logout interface unavailable during seed preparation".into());
                    }
                    prep.phase = Some(PrepPhase::WaitLogout);
                }
                PrepPhase::WaitRelog
                    if client.ingame
                        && client.scene_state == 2
                        && snapshot
                            .side_tabs()
                            .iter()
                            .any(|tab| tab.index == 3 && tab.available) =>
                {
                    prep.phase = Some(PrepPhase::ReseedTutorial);
                }
                PrepPhase::ReseedTutorial => {
                    let baseline = scenario::tutorial::chat_baseline(&snapshot);
                    send_cheat(client, scenario::tutorial::TUTORIAL_SETVAR)?;
                    send_cheat(client, scenario::tutorial::TUTORIAL_GETVAR)?;
                    prep.tutorial_reseed =
                        Some(scenario::tutorial::PostRelogTutorial::new(baseline));
                    prep.phase = Some(PrepPhase::WaitReseed);
                }
                PrepPhase::WaitReseed => {
                    let confirmed = prep
                        .tutorial_reseed
                        .as_ref()
                        .expect("post-relogin tutorial confirmation armed")
                        .check(&snapshot)?;
                    if confirmed {
                        prep.phase = Some(PrepPhase::SeedAccount);
                    }
                }
                PrepPhase::SeedAccount => {
                    send_cheat(
                        client,
                        &format!("setstat {} {}", seed.skill.name(), seed.level),
                    )?;
                    send_cheat(client, "setstat defence 98")?;
                    send_cheat(client, "setstat hitpoints 99")?;
                    send_cheat(client, &format!("give {} 1", seed.tool_alias))?;
                    send_cheat(
                        client,
                        &interact::tele_args(seed.tile.level, seed.tile.x, seed.tile.z),
                    )?;
                    prep.phase = Some(PrepPhase::WaitSeed);
                }
                PrepPhase::WaitSeed => {
                    let skill_level = snapshot
                        .stats()
                        .iter()
                        .find(|stat| stat.name.eq_ignore_ascii_case(seed.skill.name()))
                        .map(|stat| stat.base);
                    let tool_count: u64 = snapshot
                        .inventory()
                        .iter()
                        .filter(|item| i64::from(item.def.id) == i64::from(seed.tool_id))
                        .map(|item| item.count.max(0) as u64)
                        .sum();
                    let tile = snapshot.tile();
                    if client.ingame
                        && client.scene_state == 2
                        && tile == Some((seed.tile.x, seed.tile.z, seed.tile.level))
                        && skill_level == Some(i32::from(seed.level))
                        && tool_count >= 1
                    {
                        prep.evidence = Some(json!({
                            "tutorial_relogin_confirmed": true,
                            "skill": seed.skill.name(),
                            "observed_skill_level": skill_level,
                            "tool_id": seed.tool_id,
                            "observed_tool_count": tool_count,
                            "observed_tile": tile,
                            "ingame": client.ingame,
                            "scene_state": client.scene_state,
                        }));
                        prep.phase = Some(PrepPhase::Ready);
                    }
                }
                _ => {}
            }
            Ok(())
        })();
        if let Err(error) = result {
            prep.error = Some(error);
        }
    }
}

fn read_seed() -> Option<(PathBuf, GatherSeed)> {
    let path = std::env::var_os("BOT_T2_LIVE_GATHER_SEED").map(PathBuf::from)?;
    let seed = GatherSeed::parse(&std::fs::read(&path).expect("gather seed JSON"))
        .unwrap_or_else(|error| panic!("invalid gather seed {}: {error}", path.display()));
    Some((path, seed))
}

#[test]
fn gather_seed_configuration_is_closed_and_validated() {
    let valid = br#"{"skill":"woodcutting","level":1,"tool_alias":"bronze_axe","tool_id":1351,"tile":{"x":3098,"z":3242,"level":0}}"#;
    let parsed = GatherSeed::parse(valid).unwrap();
    assert_eq!(parsed.skill, GatherSkill::Woodcutting);
    assert_eq!(parsed.tool_alias, "bronze_axe");
    let mining = GatherSeed::parse(
        br#"{"skill":"mining","level":99,"tool_alias":"pickaxe","tool_id":1265,"tile":{"x":1,"z":2,"level":3}}"#,
    )
    .unwrap();
    assert_eq!(mining.skill, GatherSkill::Mining);
    let fishing = GatherSeed::parse(
        br#"{"skill":"fishing","level":1,"tool_alias":"net","tool_id":303,"tile":{"x":1,"z":2,"level":3}}"#,
    )
    .unwrap();
    assert_eq!(fishing.skill, GatherSkill::Fishing);
    assert!(GatherSeed::parse(
        br#"{"skill":"woodcutting","level":0,"tool_alias":"axe","tool_id":1,"tile":{"x":0,"z":0,"level":0}}"#
    )
    .is_err());
    assert!(GatherSeed::parse(
        br#"{"skill":"woodcutting","level":1,"tool_alias":"axe 1","tool_id":1,"tile":{"x":0,"z":0,"level":0}}"#
    )
    .is_err());
    assert!(GatherSeed::parse(
        br#"{"skill":"woodcutting","level":1,"tool_alias":"axe;tele","tool_id":1,"tile":{"x":0,"z":0,"level":0}}"#
    )
    .is_err());
    assert!(GatherSeed::parse(
        br#"{"skill":"woodcutting","level":1,"tool_alias":"axe","tool_id":1,"tile":{"x":0,"z":0,"level":0},"command":"anything"}"#
    )
    .is_err());

    assert!(GatherSeed::parse(
        br#"{"skill":"woodcutting","level":1,"tool_alias":"axe","tool_id":-1,"tile":{"x":0,"z":0,"level":0}}"#
    )
    .is_err());
    assert!(GatherSeed::parse(
        br#"{"skill":"woodcutting;tele","level":1,"tool_alias":"axe","tool_id":1,"tile":{"x":0,"z":0,"level":0}}"#
    )
    .is_err());
}

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
    let source_sha = script_js::JsCache::origin_sha(source.as_bytes());
    let expected: Value = serde_json::from_slice(
        &std::fs::read(&expected_path).expect("exact expected family result"),
    )
    .expect("expected result is JSON");
    let seed_config = read_seed();
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
    let prep = Arc::new(Mutex::new(SeedPrep::default()));
    let frame_prep = Arc::clone(&prep);
    let frame_seed = seed_config.as_ref().map(|(_, seed)| seed.clone());
    let frame_account = account.clone();
    let mut play = run_with_template(
        template,
        false,
        vec![],
        |_| (None, None),
        move |client, name, input| {
            if name == frame_account {
                if let Some(seed) = frame_seed.as_ref() {
                    seed_frame(client, input.hold, seed, &frame_prep);
                }
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
    if seed_config.is_some() {
        loop {
            let state = prep.lock().unwrap();
            if let Some(error) = state.error.as_ref() {
                panic!("gather seed preparation failed: {error}");
            }
            if state.phase == Some(PrepPhase::Ready) {
                break;
            }
            drop(state);
            assert!(
                Instant::now() < deadline,
                "gather seed preparation did not become ready: {:?}",
                prep.lock().unwrap().phase
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    } else {
        while !ingame.load(Ordering::Acquire) {
            assert!(
                Instant::now() < deadline,
                "Engine A did not reach a ready scene"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    play.script_start_load(
        account,
        source,
        script_js::LoadShape::CompatClass,
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
    let mut result = json!({
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
    if let Some((path, seed)) = seed_config.as_ref() {
        result.as_object_mut().unwrap().insert(
            "gather_seed".into(),
            json!({
                "source": path,
                "config": seed,
                "observed_preparation": prep.lock().unwrap().evidence.clone(),
                "script_logs": logs,
            }),
        );
    }
    let result_path = evidence.join("family-live-result.json");
    std::fs::write(&result_path, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    println!("family live result: {}\n{result}", result_path.display());
    drop(play);
}
