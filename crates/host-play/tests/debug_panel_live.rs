//! Explicit local-only proof of the Debug catalog's existing host cheat path.
#![cfg(feature = "debug-catalog")]
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::hostlog::{Record, Sink};
use api::snapshot::GameSnapshot;
use host::Pump;
use host_play::{ProfileClass, ProfileOptions, SharedClientTemplate};
use parking_lot::Mutex;
use vault::{Profile, ProfileSettings};

#[derive(Default, Debug)]
struct Observation {
    ready: bool,
    attack: i32,
    coins: i32,
    tile: Option<(i32, i32, i32)>,
    hitpoints: i32,
    air_runes: i32,
    quest_menu: bool,
}

#[derive(Default)]
struct ReplyLog(Mutex<Vec<(String, String)>>);
static REPLIES: LazyLock<ReplyLog> = LazyLock::new(ReplyLog::default);
impl Sink for ReplyLog {
    fn record(&self, record: &Record<'_>) {
        if record.message.starts_with("debug ::") {
            println!("{} {}", record.slot.unwrap_or("process"), record.message);
            self.0.lock().push((
                record.slot.unwrap_or_default().into(),
                record.message.into(),
            ));
        }
    }
}

fn wait_for(state: &Mutex<Observation>, label: &str, predicate: impl Fn(&Observation) -> bool) {
    let until = Instant::now() + Duration::from_secs(90);
    loop {
        {
            let state = state.lock();
            if predicate(&state) {
                println!("PASS {label}: {state:?}");
                return;
            }
            assert!(Instant::now() < until, "{label}: timed out: {state:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "requires LIVE=1 and a disposable HOME against the local engine"]
fn debug_catalog_commands_and_host_reply_log() {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return;
    }
    let engine = PathBuf::from(std::env::var_os("DEBUGPANEL_ENGINE").expect("DEBUGPANEL_ENGINE"));
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(45594),
        http_port: Some(2080),
        engine_dir: Some(engine.clone()),
        content_dir: Some(engine.parent().unwrap().join("content")),
        ..ProfileOptions::default()
    };
    let profile = options.resolve(None).unwrap().bind().unwrap();
    assert_eq!(profile.profile_class(), ProfileClass::Local);
    assert!(
        profile.game_data_status().is_attached(),
        "selected catalog: {}",
        profile.game_data_status().detail()
    );
    let game_data = profile
        .debug_catalog()
        .expect("verified Local Debug catalog");
    let template = SharedClientTemplate::load(profile).unwrap();
    let replies = &*REPLIES;
    assert!(api::hostlog::install_sink(replies));
    let state = Arc::new(Mutex::new(Observation::default()));
    let observed = Arc::clone(&state);
    let publication = Mutex::new((GameSnapshot::new(), Pump::new()));
    let mut play = host_play::run_with_template(
        template,
        false,
        vec![],
        |_| (None, None),
        move |client, _, _| {
            let mut publication = publication.lock();
            let (snapshot, pump) = &mut *publication;
            host::publish_snapshot(snapshot, client, pump.drain_client(client));
            let mut observation = observed.lock();
            observation.ready = snapshot.ingame() && snapshot.scene_state() == 2;
            observation.attack = snapshot
                .stats()
                .iter()
                .find(|stat| stat.name == "attack")
                .map_or(0, |stat| stat.base);
            observation.coins = snapshot.inv_count(995);
            observation.tile = snapshot.tile();
            observation.hitpoints = snapshot
                .stats()
                .iter()
                .find(|stat| stat.name == "hitpoints")
                .map_or(0, |stat| stat.effective);
            observation.air_runes = snapshot.inv_count(556);
            observation.quest_menu = snapshot
                .chat_modal_texts()
                .iter()
                .any(|text| text.contains("Quest Commands"));
        },
    )
    .unwrap();
    let serial = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let prefix = std::env::var("DEBUGPANEL_PREFIX").unwrap_or_else(|_| "dbg2".into());
    let user = format!("{prefix}{:06}", serial % 1_000_000);
    let slot = Profile {
        username: user.clone(),
        password: "debug-fixture".into(),
        uid: (serial % i32::MAX as u128) as i32,
        settings: ProfileSettings {
            random_events: false,
            ..ProfileSettings::default()
        },
    };
    play.spawn_slot(slot, None, None, None);
    wait_for(&state, "local ingame scene 2", |s| s.ready);
    let send = |name: &str, arguments: &[&str]| {
        let command = game_data
            .commands()
            .iter()
            .find(|command| command.name == name)
            .unwrap_or_else(|| panic!("selected content lacks {name}"));
        assert!(!command.production_only);
        let arguments = arguments
            .iter()
            .map(|value| (*value).to_string())
            .collect::<Vec<_>>();
        let wire = command.format_command(&arguments).unwrap();
        println!("SEND {}: {wire}", command.category);
        play.cheat(&user, &wire).unwrap();
    };
    send("setvar", &["tutorial", "1000"]);
    send("tele", &["0,50,50,20,20"]);
    wait_for(&state, "Teleport", |s| {
        s.ready && s.tile == Some((3220, 3220, 0))
    });
    send("~east", &["1"]);
    wait_for(&state, "Teleport content", |s| {
        s.ready && s.tile == Some((3221, 3220, 0))
    });
    send("setstat", &["attack", "42"]);
    wait_for(&state, "Account", |s| s.attack == 42);
    send("~1hp", &[]);
    wait_for(&state, "Account content", |s| s.hitpoints == 1);
    let coins = state.lock().coins;
    send("give", &["coins", "17"]);
    wait_for(&state, "Item", |s| s.coins == coins + 17);
    let air_runes = state.lock().air_runes;
    send("~giverunes", &[]);
    wait_for(&state, "Item content", |s| s.air_runes == air_runes + 1000);
    send("getcoord", &[]);
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        if replies.0.lock().iter().any(|(slot, message)| {
            slot == &user && message.contains("getcoord") && message.contains("reply candidate:")
        }) {
            break;
        }
        assert!(
            Instant::now() < until,
            "getcoord reply was not logged against command and bot"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    // Read-only quest/debug menus, not complete/reset cheats or world mutation.
    send("~quests", &[]);
    wait_for(&state, "Quest content", |s| s.quest_menu);
    play.queue_wire(&user, host_play::WireCmd::Answer(4));
    println!("PASS DebugPanel host path and reply logging for {user}");
    play.stop_slot(&user);
}
