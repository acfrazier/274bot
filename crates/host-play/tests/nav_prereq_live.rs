#![cfg(all(
    feature = "live-harness",
    feature = "test-support",
    feature = "live-probe",
    feature = "js"
))]

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::local_state::{
    AccountKey, ContentScope, DomainEdit, DomainIdentity, DomainLease, ErrorCode, LocalStatePort,
    StoreVersion, SubmitReceipt, WriteSettlement,
};
use api::prereq_knowledge::{Fact, FactSource};
use api::quest_progress::EvidenceStamp;
use api::selected::{ClientRevision, FactKey, InclusiveRange, RunKey};
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::Client;
use host_play::admission::RiskInput;
use host_play::walk_map::{ActionKind, MapContext, MapModel, WalkActionOutcome};
use host_play::{self, ProfileOptions, SharedClientTemplate, WalkArms, WalkGlobals};
use nav_planner::bank_fetch::BankRows;
use nav_planner::router::FindOptions;
use nav_planner::tile::Tile;
use nav_planner::WorldState;
use script::local_state::{SlotState, StatePort};
use script::native::{NativeTick, Script, ScriptFailure, ScriptFlow};
use serde_json::json;
use vault::{Profile, ProfileSettings};

const PREREQ_WEST: WorldTile = WorldTile {
    x: 3408,
    z: 3489,
    level: 0,
};
const PREREQ_EAST: WorldTile = WorldTile {
    x: 3423,
    z: 3485,
    level: 0,
};
const PREREQ_SWAMP: WorldTile = WorldTile {
    x: 3443,
    z: 3456,
    level: 0,
};

struct NavLogs(Mutex<Vec<(String, String)>>);

static NAV_LOGS: NavLogs = NavLogs(Mutex::new(Vec::new()));
static INSTALL_NAV_LOGS: Once = Once::new();

impl api::hostlog::Sink for NavLogs {
    fn record(&self, record: &api::hostlog::Record<'_>) {
        if matches!(record.source, api::hostlog::Source::Nav) {
            self.0.lock().expect("nav log mutex").push((
                record.slot.unwrap_or("?").to_owned(),
                record.message.to_owned(),
            ));
        }
    }
}

fn install_nav_logs() {
    INSTALL_NAV_LOGS.call_once(|| assert!(api::hostlog::install_sink(&NAV_LOGS)));
}

fn slot_nav_logs(name: &str) -> Vec<String> {
    NAV_LOGS
        .0
        .lock()
        .expect("nav log mutex")
        .iter()
        .filter(|(slot, _)| slot == name)
        .map(|(_, message)| message.clone())
        .collect()
}

struct NoStore;

impl LocalStatePort for NoStore {
    fn lease(&self, identity: &DomainIdentity) -> Result<DomainLease, ErrorCode> {
        Err(ErrorCode::NotReady {
            domain: identity.domain(),
        })
    }

    fn submit(&self, edit: DomainEdit) -> SubmitReceipt {
        SubmitReceipt::Refused(ErrorCode::Busy {
            domain: edit.identity().domain(),
        })
    }

    fn observe(&self, _: api::identity::RequestId) -> Option<WriteSettlement> {
        None
    }
}

impl StatePort for NoStore {
    fn lease_version(&self, identity: &DomainIdentity) -> Result<StoreVersion, ErrorCode> {
        self.lease(identity).map(|lease| lease.version())
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("nav-prereq-owner-{}-{serial}", std::process::id()));
        fs::create_dir_all(&path).expect("create fixture scratch directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap_or_else(|error| panic!("create {}: {error}", to.display()));
    for entry in
        fs::read_dir(from).unwrap_or_else(|error| panic!("read {}: {error}", from.display()))
    {
        let entry = entry.expect("cache entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("cache entry type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target)
                .unwrap_or_else(|error| panic!("copy {}: {error}", entry.path().display()));
        }
    }
}

fn required_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name}")))
}

fn required_port(name: &str) -> u16 {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("set {name}"))
        .parse()
        .unwrap_or_else(|_| panic!("{name} must be a port"))
}

fn quest_varp(quests: &api::quest_facts::QuestCatalog, quest: &str) -> String {
    quests
        .iter()
        .find(|facts| facts.id.0.eq_ignore_ascii_case(quest))
        .unwrap_or_else(|| panic!("selected R289 quest facts lack quest {quest:?}"))
        .varp
        .0
        .to_string()
}

fn live_globals(dialogue: bool, quest_start: bool) -> WalkGlobals {
    // Serde keeps this test source compatible with the shared WalkGlobals
    // projection while exercising the two persisted consent fields.
    let mut value = serde_json::to_value(WalkGlobals::default()).expect("serialize walk globals");
    assert!(
        value.get("allow_prereq_dialogue").is_some(),
        "WalkGlobals must expose allow_prereq_dialogue"
    );
    assert!(
        value.get("allow_quest_start").is_some(),
        "WalkGlobals must expose allow_quest_start"
    );
    value["allow_prereq_dialogue"] = json!(dialogue);
    value["allow_quest_start"] = json!(quest_start);
    serde_json::from_value(value).expect("decode prerequisite grants")
}

fn range(value: i32) -> InclusiveRange {
    InclusiveRange {
        min: Some(value),
        max: Some(value),
    }
}

fn run_receipt(run: RunKey) -> serde_json::Value {
    json!({"slot": run.slot, "run": run.run, "session": run.session})
}

fn assert_holy_barrier_refusal_fact(fact: &Fact) {
    assert_eq!(fact.quest.0.as_ref(), "priestperil");
    assert_eq!(fact.signal.0.as_ref(), "priestperil");
    assert_eq!(fact.possible, range(60));
    assert_eq!(
        fact.after,
        InclusiveRange {
            min: Some(60),
            max: Some(61),
        }
    );
    assert_eq!(fact.refused, Some(range(61)));
}

fn wrong_memory(quest: &str, run: RunKey, stage: i32) -> Fact {
    Fact {
        quest: FactKey::new(quest),
        signal: FactKey::new(quest),
        possible: range(stage),
        after: range(stage),
        refused: None,
        // This is deliberately seeded fixture memory, not evidence from this
        // live server run. The refusal must replace it before any repair path.
        source: FactSource::Page { row: 0, line: 0 },
        observed: EvidenceStamp {
            run,
            tick: 0,
            sequence: 0,
        },
    }
}

struct CompleteOnce;

impl Script for CompleteOnce {
    fn tick(&mut self, _: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        Ok(ScriptFlow::Complete)
    }
}

enum FixtureSeedPhase {
    InitialReady,
    InitialTutorial(i32),
    WaitInitialTutorial(i32),
    WaitLogout,
    WaitRelog,
    ReseedTutorial,
    WaitReseed(scenario::tutorial::PostRelogTutorial),
    SeedPosition,
    SeedAccount(usize),
    WaitAccountSeed(i32),
    Ready,
}

fn send_seed_command(client: &mut Client, command: &str) -> Result<(), String> {
    if interact::cheat(client, command).is_sent() {
        Ok(())
    } else {
        Err(format!("refused seed command {command:?}"))
    }
}

struct LiveFixture {
    play: host_play::Play,
    profile: Arc<host_play::ServerProfile>,
    template: Arc<SharedClientTemplate>,
    state: Arc<SlotState>,
    selected: Arc<api::game_data::SelectedGameData>,
    quests: Arc<api::quest_facts::QuestCatalog>,
    snapshot: Arc<Mutex<GameSnapshot>>,
    manual_arms: WalkArms,
    name: String,
    seed_run: RunKey,
    _scratch: Scratch,
}

fn wait_for<T>(
    play: &host_play::Play,
    name: &str,
    label: &str,
    timeout: Duration,
    mut ready: impl FnMut(&host_play::Play) -> Option<T>,
) -> T {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = ready(play) {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {label}: account={name}, status={:?}, state={:?}, lifecycle={:?}, error={:?}",
            play.statuses().into_iter().find(|row| row.username == name),
            play.script_state(name),
            play.script_lifecycle_receipt(name),
            play.script_last_error(name)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn make_fixture(
    origin: WorldTile,
    server_stages: &[(&str, i32)],
    dialogue: bool,
    quest_start: bool,
) -> LiveFixture {
    make_fixture_with_seed_commands(origin, server_stages, dialogue, quest_start, &[])
}

fn make_fixture_with_seed_commands(
    origin: WorldTile,
    server_stages: &[(&str, i32)],
    dialogue: bool,
    quest_start: bool,
    seed_commands: &[String],
) -> LiveFixture {
    install_nav_logs();
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));

    let scratch = Scratch::new();
    let cache_source = required_path("BOT_CACHE_DIR");
    assert!(cache_source.is_dir(), "BOT_CACHE_DIR must be a directory");
    let cache = scratch.path().join("cache");
    copy_dir(&cache_source, &cache);

    let catalog_root = required_path("RS2B0T");
    let engine_dir = required_path("WORLD_ENGINE_DIR");
    let nav_pack = required_path("WORLD_NAV_PACK");
    assert!(catalog_root.is_dir(), "RS2B0T must be a directory");
    assert!(engine_dir.is_dir(), "WORLD_ENGINE_DIR must be a directory");
    assert!(nav_pack.is_file(), "WORLD_NAV_PACK must be a file");

    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(required_port("WORLD_GAME_PORT")),
        http_port: Some(required_port("WORLD_HTTP_PORT")),
        engine_dir: Some(engine_dir),
        cache_dir: Some(cache),
        unpack_dir: Some(scratch.path().join("unpack")),
        vault_path: Some(scratch.path().join("vault")),
        nav_pack: Some(nav_pack),
        catalog_root: Some(catalog_root),
        ..ProfileOptions::default()
    };
    let profile = options
        .resolve(None)
        .expect("resolve local-289 profile")
        .bind_runtime()
        .expect("bind local-289 runtime profile");
    let template = SharedClientTemplate::load(Arc::clone(&profile))
        .expect("load selected local-289 client template");
    assert!(
        template.world().is_some(),
        "selected navigation world is required"
    );

    let selected =
        api::game_data::for_revision(ClientRevision::R289).expect("selected R289 game data");
    let clock = api::work::SystemClock;
    let cancellation = api::work::SharedCancellation::new();
    let budget = api::work::SliceBudget::new(
        std::num::NonZeroU32::new(1).expect("nonzero quest preparation budget"),
        api::identity::Deadline::at(Instant::now() + Duration::from_secs(90)),
    );
    let mut preparation =
        api::work::StepContext::<api::work::PreparationPhase>::new(&clock, budget, &cancellation);
    let quests = api::selected::FamilyPreparation::within(&mut preparation, |worker| {
        selected.prepare_quests(worker)
    });
    let quests = quests.expect("prepare selected R289 quest facts");
    let mut frame_commands = server_stages
        .iter()
        .map(|(quest, stage)| {
            if *quest == "priestperil" {
                // Raw setvar leaves the login-time quest colour stale.
                // The content's own seed proc updates both the stage and
                // its real quest-list colour: debug_quests.rs2:327-331.
                format!("~setpp {stage}")
            } else {
                format!("setvar {} {stage}", quest_varp(&quests, quest))
            }
        })
        .collect::<Vec<_>>();
    frame_commands.extend(seed_commands.iter().cloned());
    let seed_name = host_play::mint_live_names(1)
        .pop()
        .expect("one unique live account");
    let uid = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos()
        % i32::MAX as u128) as i32;
    let account = Profile {
        username: seed_name.clone(),
        password: seed_name.clone().into(),
        uid,
        settings: ProfileSettings::default(),
    };
    let seeded = Arc::new(AtomicBool::new(false));
    let snapshot = Arc::new(Mutex::new(GameSnapshot::new()));
    let frame_snapshot = Arc::clone(&snapshot);
    let frame_seeded = Arc::clone(&seeded);
    let frame_name = seed_name.clone();
    let seed_errors = Arc::new(Mutex::new(Vec::<String>::new()));
    let frame_errors = Arc::clone(&seed_errors);
    let frame_phase = Mutex::new(FixtureSeedPhase::InitialReady);
    let manual_arms = WalkArms::default();
    let frame_manual_arms = Arc::clone(&manual_arms);
    let frame_manual_latch = Mutex::new(None);
    let frame_world = template.world().expect("selected navigation world");
    let frame_map_members = profile.map_members();
    let mut play = host_play::run_with_template(
        Arc::clone(&template),
        false,
        vec![account],
        |_| (None, None),
        move |client: &mut Client, username, input| {
            if username != frame_name {
                return;
            }
            let mut snapshot = frame_snapshot.lock().expect("game snapshot mutex");
            snapshot.rebuild(client);
            let mut phase = frame_phase.lock().expect("fixture seed phase mutex");
            if !frame_errors.lock().expect("seed error mutex").is_empty() {
                return;
            }
            if matches!(*phase, FixtureSeedPhase::Ready) {
                if input.hold || input.host_move_owned || !client.ingame || client.scene_state != 2
                {
                    return;
                }
                let arm = frame_manual_arms
                    .lock()
                    .expect("manual arms mutex")
                    .get(username)
                    .cloned();
                if let (Some(arm), Some(here)) = (arm, snapshot.tile()) {
                    let step = (client.gens.player, here);
                    let mut latch = frame_manual_latch.lock().expect("manual step latch mutex");
                    if *latch != Some(step) {
                        *latch = Some(step);
                        host_play::step_walk_arm_follow(
                            client,
                            &snapshot,
                            &mut arm.lock().expect("manual arm mutex"),
                            Some(&frame_world),
                            here,
                            frame_map_members,
                            Some(username),
                        );
                    }
                }
                return;
            }
            if matches!(*phase, FixtureSeedPhase::WaitLogout) && !client.ingame {
                *phase = FixtureSeedPhase::WaitRelog;
                return;
            }
            if input.hold || !client.ingame || client.scene_state != 2 {
                return;
            }
            let result = (|| -> Result<(), String> {
                match &*phase {
                    FixtureSeedPhase::InitialReady => {
                        client.draw = false;
                        let baseline = scenario::tutorial::chat_baseline(&snapshot);
                        interact::mainland_hop(client);
                        *phase = FixtureSeedPhase::InitialTutorial(baseline);
                    }
                    FixtureSeedPhase::InitialTutorial(baseline) => {
                        send_seed_command(client, scenario::tutorial::TUTORIAL_GETVAR)?;
                        *phase = FixtureSeedPhase::WaitInitialTutorial(*baseline);
                    }
                    FixtureSeedPhase::WaitInitialTutorial(baseline)
                        if scenario::tutorial::tutorial_confirmed(&snapshot, *baseline) =>
                    {
                        let ifaces = Arc::clone(&client.ifaces);
                        if !interact::logout(client, &ifaces) {
                            return Err(
                                "logout interface unavailable during seed preparation".into()
                            );
                        }
                        *phase = FixtureSeedPhase::WaitLogout;
                    }
                    FixtureSeedPhase::WaitRelog
                        if snapshot
                            .side_tabs()
                            .iter()
                            .any(|tab| tab.index == 3 && tab.available) =>
                    {
                        *phase = FixtureSeedPhase::ReseedTutorial;
                    }
                    FixtureSeedPhase::ReseedTutorial => {
                        let baseline = scenario::tutorial::chat_baseline(&snapshot);
                        send_seed_command(client, scenario::tutorial::TUTORIAL_SETVAR)?;
                        send_seed_command(client, scenario::tutorial::TUTORIAL_GETVAR)?;
                        *phase = FixtureSeedPhase::WaitReseed(
                            scenario::tutorial::PostRelogTutorial::new(baseline),
                        );
                    }
                    FixtureSeedPhase::WaitReseed(tutorial) => {
                        if tutorial.check(&snapshot)? {
                            println!("{}", scenario::tutorial::confirmation_log());
                            *phase = FixtureSeedPhase::SeedPosition;
                        }
                    }
                    // Queue each seed packet only after the previous frame output has flushed.
                    FixtureSeedPhase::SeedPosition if client.out.pos == 0 => {
                        send_seed_command(
                            client,
                            &interact::tele_args(origin.level, origin.x, origin.z),
                        )?;
                        *phase = FixtureSeedPhase::SeedAccount(0);
                    }
                    FixtureSeedPhase::SeedAccount(next_command) if client.out.pos == 0 => {
                        if let Some(command) = frame_commands.get(*next_command) {
                            send_seed_command(client, command)?;
                            *phase = FixtureSeedPhase::SeedAccount(*next_command + 1);
                        } else {
                            let baseline = scenario::tutorial::chat_baseline(&snapshot);
                            send_seed_command(client, scenario::tutorial::TUTORIAL_GETVAR)?;
                            *phase = FixtureSeedPhase::WaitAccountSeed(baseline);
                        }
                    }
                    FixtureSeedPhase::WaitAccountSeed(baseline) => {
                        if let Some(line) = snapshot.chat_lines().iter().find(|line| {
                            line.sequence > *baseline
                                && scenario::tutorial::parse_tutorial_reply(&line.text)
                                    == Some(scenario::tutorial::TUTORIAL_COMPLETE)
                        }) {
                            println!(
                                "fixture account seeds confirmed: tick={} sequence={} after={}",
                                snapshot.tick(),
                                line.sequence,
                                baseline
                            );
                            *phase = FixtureSeedPhase::Ready;
                            frame_seeded.store(true, Ordering::Release);
                        }
                    }
                    _ => {}
                }
                Ok(())
            })();
            if let Err(error) = result {
                frame_errors.lock().expect("seed error mutex").push(error);
            }
        },
    )
    .expect("start isolated local-289 Play");
    play.focus(&seed_name);
    play.set_walk_globals(live_globals(dialogue, quest_start));

    wait_for(
        &play,
        &seed_name,
        "real server seed position",
        Duration::from_secs(240),
        |play| {
            let errors = seed_errors.lock().expect("seed error mutex");
            assert!(errors.is_empty(), "fixture seed errors: {errors:?}");
            if !seeded.load(Ordering::Acquire) {
                return None;
            }
            play.statuses().into_iter().find(|row| {
                row.username == seed_name
                    && row.ingame
                    && row.scene_state == 2
                    && row.tile_x == origin.x
                    && row.tile_z == origin.z
                    && row.tile_level == origin.level
            })
        },
    );

    for (quest, stage) in server_stages {
        let varp = quest_varp(&quests, quest);
        play.cheat(&seed_name, &format!("getvar {varp}"))
            .expect("queue live server stage query");
        wait_for(
            &play,
            &seed_name,
            "observed server quest stage",
            Duration::from_secs(30),
            |play| {
                play.statuses().into_iter().find(|row| {
                    row.username == seed_name
                        && row.chat_head.contains(&format!("get {varp}: {stage}"))
                })
            },
        );
    }

    let state = Arc::new(SlotState::new(
        Arc::new(NoStore),
        AccountKey::try_new("local-289", &seed_name).expect("session account key"),
        ContentScope::try_new(ClientRevision::R289, "nav-prereq-owner-live")
            .expect("session content scope"),
    ));
    let seed_run = play
        .script_start_handle()
        .start_test_script_with_local_state(
            &seed_name,
            Box::new(CompleteOnce),
            Some(Arc::clone(&selected)),
            Arc::clone(&state),
        )
        .expect("bind session-local state before first script tick");
    wait_for(
        &play,
        &seed_name,
        "one-shot local-state binder completion",
        Duration::from_secs(30),
        |play| {
            play.script_lifecycle_receipt(&seed_name)
                .filter(|receipt| receipt.state == script::ScriptTerminalState::Completed)
        },
    );
    let session = play
        .arm(&seed_name)
        .and_then(|arm| arm.session_key())
        .expect("fixture binder belongs to the actual live host session");
    assert_eq!(
        session.slot().incarnation().get(),
        seed_run.slot,
        "fixture binder keeps its slot incarnation"
    );
    let seed_run = RunKey {
        session: session.epoch(),
        ..seed_run
    };

    LiveFixture {
        play,
        profile,
        template,
        state,
        selected,
        quests,
        snapshot,
        manual_arms,
        name: seed_name,
        seed_run,
        _scratch: scratch,
    }
}

fn seed_wrong_memory(fixture: &LiveFixture, quest: &str, stage: i32, run: RunKey) {
    assert!(fixture
        .state
        .record_prereq_fact(wrong_memory(quest, run, stage)));
    let fact = fixture.state.prereq_snapshot().facts().first().cloned();
    assert!(fact.is_some_and(|fact| {
        fact.quest.0.as_ref() == quest
            && fact.possible == range(stage)
            && fact.refused.is_none()
            && fact.observed.run == run
    }));
}

fn refusal_fact(fixture: &LiveFixture, quest: &str, timeout: Duration) -> Fact {
    wait_for(
        &fixture.play,
        &fixture.name,
        "server-authored GateRefused fact",
        timeout,
        |_| {
            fixture
                .state
                .prereq_snapshot()
                .facts()
                .iter()
                .find(|fact| {
                    fact.quest.0.as_ref() == quest
                        && fact.refused.is_some()
                        && matches!(&fact.source, FactSource::Refusal { .. })
                })
                .cloned()
        },
    )
}

fn assert_no_repair_waits(name: &str) -> (Vec<String>, Vec<String>) {
    let logs = slot_nav_logs(name);
    let waits = logs
        .iter()
        .filter(|line| {
            [
                "prereq wait-start=W2",
                "prereq wait-start=W3",
                "prereq wait-start=W4",
                "prereq wait-start=W5",
                "prereq wait=W2",
                "prereq wait=W3",
                "prereq wait=W4",
                "prereq wait=W5",
            ]
            .iter()
            .any(|&prefix| line.contains(prefix))
        })
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        waits.is_empty(),
        "consent denial started a repair wait: {waits:?}"
    );
    assert_refusal_record_precedes_handover(&logs);
    (logs, waits)
}

fn assert_refusal_record_precedes_handover(logs: &[String]) {
    let record = logs
        .iter()
        .position(|line| line.contains("prereq refusal-record "))
        .unwrap_or_else(|| panic!("missing refusal record receipt: {logs:?}"));
    let handover = logs
        .iter()
        .position(|line| line.contains("prereq handover ") && line.contains("entry=GateRefused"))
        .unwrap_or_else(|| panic!("missing GateRefused handover receipt: {logs:?}"));
    assert!(
        record < handover,
        "handover preceded the refusal record: {logs:?}"
    );
}

fn nav_tile(tile: WorldTile) -> Tile {
    Tile {
        x: tile.x,
        z: tile.z,
        level: tile.level,
    }
}

fn map_context(fixture: &LiveFixture) -> MapContext {
    let pack_id = fixture
        .profile
        .nav_pack_id()
        .expect("bound navigation pack ownership id");
    MapContext {
        focus: fixture.play.map_focus(&fixture.name),
        nav: pack_id,
        overlay: None,
        generation: 1,
    }
}

fn manual_map_walk(fixture: &LiveFixture, destination: WorldTile) -> serde_json::Value {
    let world = fixture.template.world().expect("selected navigation world");
    let context = map_context(fixture);
    let origin = fixture
        .play
        .statuses()
        .into_iter()
        .find(|row| row.username == fixture.name)
        .expect("live slot status");
    assert!(origin.ingame && origin.scene_state == 2);
    let origin_tile = Tile {
        x: origin.tile_x,
        z: origin.tile_z,
        level: origin.tile_level,
    };
    let mut model = MapModel::default();
    model.bind(context);
    let selected = model.select_tile(&world, nav_tile(destination));
    assert_eq!(
        selected,
        Some(nav_tile(destination)),
        "prerequisite destination stand"
    );
    let command = model
        .confirm(
            ActionKind::Walk,
            &context,
            Some(origin_tile),
            FindOptions::default(),
        )
        .expect("confirm real manual map destination");
    let state = WorldState::empty().with_map_members(fixture.profile.map_members());
    let snapshot = fixture.snapshot.lock().expect("game snapshot mutex");
    let outcome = fixture
        .play
        .map_walk(
            command,
            &context,
            &snapshot,
            &state,
            &BankRows::default(),
            RiskInput::default(),
            &fixture.manual_arms,
        )
        .expect("arm owner-driven manual route");
    let admission = match outcome {
        WalkActionOutcome::Walking(route) => {
            let arm = fixture
                .manual_arms
                .lock()
                .expect("manual arms mutex")
                .get(&fixture.name)
                .cloned()
                .expect("accepted manual route arm");
            assert!(
                arm.lock()
                    .expect("manual arm mutex")
                    .manual_prereq
                    .is_some(),
                "accepted prerequisite route must retain its crossing content"
            );
            json!({
                "kind": "walking",
                "route_ticks": route.route.ticks,
                "route_legs": format!("{:?}", route.route.legs),
                "assessment": format!("{:?}", route.assessment.verdict),
            })
        }
        WalkActionOutcome::PrerequisiteStarted => json!({ "kind": "prerequisite-started" }),
    };
    json!({
        "origin": [origin_tile.x, origin_tile.z, origin_tile.level],
        "destination": [destination.x, destination.z, destination.level],
        "admission": admission,
    })
}

fn assert_not_allowed_terminal(
    fixture: &LiveFixture,
    timeout: Duration,
) -> script::ScriptLifecycleReceipt {
    let terminal = wait_for(
        &fixture.play,
        &fixture.name,
        "registered WalkTo NotAllowed terminal",
        timeout,
        |play| {
            play.script_lifecycle_receipt(&fixture.name)
                .filter(|receipt| receipt.state == script::ScriptTerminalState::Failed)
        },
    );
    assert!(
        terminal.reason.contains("NotAllowed"),
        "WalkTo terminal is not the consent denial: {terminal:?}"
    );
    terminal
}

const FULL_BANK_SLOTS: usize = 288;
const COMPAT_RESULT_MARKER: &str = "NAV_PREREQ_RESULT ";

fn safe_item_alias(alias: &str) -> bool {
    !alias.is_empty()
        && alias
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn item_id(selected: &api::game_data::SelectedGameData, alias: &str) -> i32 {
    selected
        .item_by_alias(alias)
        .unwrap_or_else(|| panic!("selected R289 data has no item {alias:?}"))
        .id
}

fn inventory_seed_aliases(
    selected: &api::game_data::SelectedGameData,
    slots: usize,
) -> Vec<String> {
    let mut aliases = Vec::with_capacity(slots);
    let mut ids = HashSet::with_capacity(slots);
    for item in selected.items() {
        if aliases.len() == slots {
            break;
        }
        if item.id <= 0 || item.stackable || item.stack_variant {
            continue;
        }
        let Some(alias) = item.alias.as_deref() else {
            continue;
        };
        if !safe_item_alias(alias) || matches!(alias, "meat_pie" | "apple_pie" | "dagger_wolfbane")
        {
            continue;
        }
        if ids.insert(item.id) {
            aliases.push(alias.to_owned());
        }
    }
    assert_eq!(
        aliases.len(),
        slots,
        "selected R289 data must provide the requested distinct pack items"
    );
    aliases
}

fn item_seed_commands(verb: &str, aliases: &[String]) -> Vec<String> {
    aliases
        .iter()
        .map(|alias| format!("{verb} {alias} 1"))
        .collect()
}

fn full_bank_seed_commands(
    selected: &api::game_data::SelectedGameData,
    pack_aliases: &[String],
) -> Vec<String> {
    let mut excluded = HashSet::with_capacity(pack_aliases.len() + 2);
    for alias in pack_aliases
        .iter()
        .map(String::as_str)
        .chain(["meat_pie", "apple_pie"])
    {
        if let Some(item) = selected.item_by_alias(alias) {
            excluded.insert(item.id);
        }
    }
    let mut ids = HashSet::with_capacity(FULL_BANK_SLOTS);
    let mut commands = Vec::with_capacity(FULL_BANK_SLOTS);
    let dagger = selected
        .item_by_alias("dagger_wolfbane")
        .expect("selected R289 data has Wolfbane");
    assert!(!excluded.contains(&dagger.id), "Wolfbane must be banked");
    ids.insert(dagger.id);
    commands.push("givebank dagger_wolfbane 1".to_owned());
    for item in selected.items() {
        let Some(alias) = item.alias.as_deref() else {
            continue;
        };
        if item.id <= 0
            || item.stack_variant
            || excluded.contains(&item.id)
            || !safe_item_alias(alias)
            || !ids.insert(item.id)
        {
            continue;
        }
        commands.push(format!("givebank {alias} 1"));
        if commands.len() == FULL_BANK_SLOTS {
            break;
        }
    }
    assert_eq!(
        commands.len(),
        FULL_BANK_SLOTS,
        "selected R289 data must provide 288 distinct bank item ids"
    );
    commands
}

fn item_count(items: &[api::snapshot::ItemView], id: i32) -> i32 {
    items
        .iter()
        .filter(|item| item.def.id == id)
        .map(|item| item.count)
        .sum()
}

fn wait_for_snapshot<T>(
    fixture: &LiveFixture,
    label: &str,
    timeout: Duration,
    mut ready: impl FnMut(&GameSnapshot) -> Option<T>,
) -> T {
    wait_for(&fixture.play, &fixture.name, label, timeout, |_| {
        let snapshot = fixture.snapshot.lock().expect("game snapshot mutex");
        ready(&snapshot)
    })
}

fn wait_for_inventory_slots(fixture: &LiveFixture, slots: usize) -> u32 {
    wait_for_snapshot(
        fixture,
        "observed seeded inventory slots",
        Duration::from_secs(45),
        |snapshot| {
            (snapshot.inventory_size() == 28 && snapshot.inventory().len() == slots)
                .then_some(snapshot.tick())
        },
    )
}

fn start_compat(fixture: &LiveFixture, source: String) -> RunKey {
    let run = fixture.seed_run.next_run().expect("next compat run key");
    fixture
        .play
        .script_start_load(
            &fixture.name,
            source,
            script_js::LoadShape::CompatClass,
            None,
            vec![],
        )
        .expect("start prerequisite compat WalkTo");
    let mut settled = None;
    wait_for(
        &fixture.play,
        &fixture.name,
        "compat Start outcome",
        Duration::from_secs(30),
        |play| {
            if let script::StartPoll::Settled(outcome) = play.script_poll_start(&fixture.name) {
                settled = Some(outcome);
                Some(())
            } else {
                None
            }
        },
    );
    assert_eq!(settled, Some(script::StartOutcome::Ready));
    let session = fixture
        .play
        .arm(&fixture.name)
        .and_then(|arm| arm.session_key())
        .expect("compat belongs to the actual live host session");
    assert_eq!(
        session.slot().incarnation().get(),
        run.slot,
        "compat keeps the fixture's slot incarnation"
    );
    RunKey {
        session: session.epoch(),
        ..run
    }
}

fn wait_compat_result(
    fixture: &LiveFixture,
    timeout: Duration,
) -> (serde_json::Value, Vec<String>) {
    let mut logs = Vec::new();
    wait_for(
        &fixture.play,
        &fixture.name,
        "compat WalkTo result",
        timeout,
        |play| {
            logs.extend(play.script_take_pending_logs(&fixture.name));
            logs.iter()
                .any(|line| line.contains(COMPAT_RESULT_MARKER))
                .then_some(())
        },
    );
    let line = logs
        .iter()
        .find(|line| line.contains(COMPAT_RESULT_MARKER))
        .expect("compat result marker");
    let payload = line
        .split_once(COMPAT_RESULT_MARKER)
        .expect("compat result marker separator")
        .1;
    (
        serde_json::from_str(payload).expect("decode compat result receipt"),
        logs,
    )
}

fn stop_compat(fixture: &LiveFixture) {
    fixture.play.script_stop(&fixture.name);
    wait_for(
        &fixture.play,
        &fixture.name,
        "compat script stop",
        Duration::from_secs(30),
        |play| (play.script_state(&fixture.name) == script::RunState::Idle).then_some(()),
    );
}

fn assert_seat_idle(fixture: &LiveFixture) -> serde_json::Value {
    assert_eq!(
        fixture.play.script_live_walk_request_id(&fixture.name),
        None,
        "compat walk owner remains live"
    );
    let probe = fixture
        .play
        .script_seat_probe(&fixture.name)
        .expect("compat walk seat exists");
    assert!(!probe.held, "compat seat still holds an action: {probe:?}");
    assert_eq!(probe.live, 0, "compat seat has live rows: {probe:?}");
    assert_eq!(
        probe.results, 0,
        "compat seat has unconsumed results: {probe:?}"
    );
    assert_eq!(
        probe.reserved_releases, 0,
        "compat seat has reserved releases: {probe:?}"
    );
    json!({
        "held": probe.held,
        "dropped": probe.dropped,
        "late_controls": probe.late_controls,
        "live": probe.live,
        "results": probe.results,
        "reserved_releases": probe.reserved_releases,
        "mirror_revokes_consumed": probe.mirror_revokes_consumed,
    })
}

fn wait_for_manual_walk_run(fixture: &LiveFixture) -> RunKey {
    wait_for(
        &fixture.play,
        &fixture.name,
        "manual WalkTo card start",
        Duration::from_secs(30),
        |play| {
            // The internal WalkTo card emits no ScriptStatus.
            // Require its new native owner, not an optional status payload.
            let session = play.arm(&fixture.name)?.session_key()?;
            play.script_native_run(&fixture.name).filter(|run| {
                run.slot == fixture.seed_run.slot
                    && run.run > fixture.seed_run.run
                    && run.session_key() == Some(session)
            })
        },
    )
}

fn lifecycle_generation(fixture: &LiveFixture) -> u64 {
    fixture
        .play
        .script_lifecycle_receipt(&fixture.name)
        .expect("prior script lifecycle receipt")
        .runtime_generation
}

fn wait_for_manual_terminal(
    fixture: &LiveFixture,
    run: RunKey,
    previous_generation: u64,
    state: script::ScriptTerminalState,
    timeout: Duration,
) -> script::ScriptLifecycleReceipt {
    wait_for(
        &fixture.play,
        &fixture.name,
        "manual WalkTo terminal",
        timeout,
        |play| {
            play.script_lifecycle_receipt(&fixture.name)
                .filter(|receipt| {
                    receipt.state == state
                        && receipt.runtime_generation > previous_generation
                        && play
                            .script_native_run(&fixture.name)
                            .is_none_or(|current| current == run)
                })
        },
    )
}

fn wait_for_tile(fixture: &LiveFixture, destination: WorldTile) -> serde_json::Value {
    wait_for(
        &fixture.play,
        &fixture.name,
        "server position at prerequisite destination",
        Duration::from_secs(180),
        |play| {
            let row = play.statuses().into_iter().find(|row| {
                row.username == fixture.name
                    && row.ingame
                    && row.scene_state == 2
                    && row.tile_x == destination.x
                    && row.tile_z == destination.z
                    && row.tile_level == destination.level
            })?;
            Some(json!({
                "tile": [row.tile_x, row.tile_z, row.tile_level],
                "scene_state": row.scene_state,
            }))
        },
    )
}

fn compat_holy_source(destination: WorldTile, options: &str, second_walk: bool) -> String {
    let options = if options.is_empty() {
        String::new()
    } else {
        format!(", {options}")
    };
    let second_call = if second_walk {
        format!(
            r#"
        const second = await Traversal.walkTo(
            {{ x: {}, z: {}, level: {} }},
            options
        );
        globalThis.__nav_prereq_terminal_count += 1;
        globalThis.__nav_prereq_second_result = second;
"#,
            destination.x, destination.z, destination.level
        )
    } else {
        String::new()
    };
    format!(
        r#"
import {{ Traversal }} from '../../api/walking/Traversal.js';
export default class PrereqHolyCompat extends LoopingBot {{
    async loop() {{
        if (globalThis.__nav_prereq_started) return;
        globalThis.__nav_prereq_started = true;
        globalThis.__nav_prereq_terminal_count = 0;
        const options = {{ radius: 0{options} }};
        const first = await Traversal.walkTo(
            {{ x: {}, z: {}, level: {} }},
            options
        );
        globalThis.__nav_prereq_terminal_count += 1;
        globalThis.__nav_prereq_first_result = first;
        {second_call}
        this.log("NAV_PREREQ_RESULT " + JSON.stringify({{
            terminal_count: globalThis.__nav_prereq_terminal_count,
            first: globalThis.__nav_prereq_first_result,
            second: globalThis.__nav_prereq_second_result ?? null
        }}));
    }}
}}
"#,
        destination.x, destination.z, destination.level
    )
}
#[test]
#[ignore = "requires LIVE=1 and the explicitly configured local Engine L on 47594/3080"]
fn live_manual_prereq_holy_barrier_denied() {
    if !api::test_support::game_data_available() {
        return;
    }
    let fixture = make_fixture(
        PREREQ_WEST,
        &[("priestperil", 60), ("druidspirit", 0)],
        false,
        true,
    );
    seed_wrong_memory(&fixture, "priestperil", 61, fixture.seed_run);
    let route = manual_map_walk(&fixture, PREREQ_EAST);
    let fact = refusal_fact(&fixture, "priestperil", Duration::from_secs(180));
    assert_holy_barrier_refusal_fact(&fact);
    let terminal = assert_not_allowed_terminal(&fixture, Duration::from_secs(180));
    assert_eq!(
        fixture.play.script_native_run(&fixture.name),
        None,
        "first manual WalkTo owner remains live after its terminal"
    );
    let second_route = manual_map_walk(&fixture, PREREQ_EAST);
    let second_terminal = wait_for_manual_terminal(
        &fixture,
        fact.observed.run,
        terminal.runtime_generation,
        script::ScriptTerminalState::Failed,
        Duration::from_secs(180),
    );
    assert!(
        second_terminal.reason.contains("NotAllowed"),
        "second same-run manual WalkTo did not retain the consent denial: {second_terminal:?}"
    );
    assert_eq!(
        fixture.play.script_native_run(&fixture.name),
        None,
        "second manual WalkTo owner remains live after its terminal"
    );
    let retained_fact = refusal_fact(&fixture, "priestperil", Duration::from_secs(30));
    assert_eq!(
        retained_fact.observed, fact.observed,
        "second same-run manual WalkTo retried the refused crossing"
    );
    assert_eq!(retained_fact.possible, fact.possible);
    assert_eq!(retained_fact.refused, fact.refused);
    let (nav_logs, repair_waits) = assert_no_repair_waits(&fixture.name);
    let receipt = json!({
        "cell": "A1-manual-holy-barrier-denied",
        "account": fixture.name,
        "server_stage": {"priestperil": 60, "druidspirit": 0},
        "global_allow_prereq_dialogue": false,
        "global_allow_quest_start": true,
        "wrong_memory": {"priestperil": 61, "run": run_receipt(fixture.seed_run)},
        "server_refusal_fact": format!("{fact:?}"),
        "manual_terminal_refusal": terminal,
        "route": route,
        "second_manual_terminal_refusal": second_terminal,
        "second_route": second_route,
        "same_refusal_observation_after_second_walk": format!("{:?}", retained_fact.observed),
        "navigation_logs": nav_logs,
        "repair_waits": repair_waits,
        "session_only": true,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&receipt).expect("serialize A1 receipt")
    );
    println!("prereq cell outcome=completed test=live_manual_prereq_holy_barrier_denied");
}

#[test]
#[ignore = "requires LIVE=1 and the explicitly configured local Engine L on 47594/3080"]
fn live_compat_prereq_mort_myre_denied() {
    if !api::test_support::game_data_available() {
        return;
    }
    let fixture = make_fixture(
        PREREQ_EAST,
        &[("priestperil", 61), ("druidspirit", 0)],
        true,
        true,
    );
    let expected_run = fixture.seed_run.next_run().expect("next compat run key");
    seed_wrong_memory(&fixture, "druidspirit", 10, expected_run);
    fixture
        .play
        .script_start_load(
            &fixture.name,
            compat_holy_source(PREREQ_SWAMP, "allowQuestStart: false", false),
            script_js::LoadShape::CompatClass,
            None,
            vec![],
        )
        .expect("start compat WalkTo with request-level Forbid");

    let fact = refusal_fact(&fixture, "druidspirit", Duration::from_secs(240));
    assert_eq!(
        fact.observed.run, expected_run,
        "refusal belongs to compat run"
    );
    let (result, logs) = wait_compat_result(&fixture, Duration::from_secs(240));
    assert_eq!(
        result["terminal_count"], 1,
        "expected one outer WalkTo terminal"
    );
    assert_eq!(
        result["first"].as_bool(),
        Some(false),
        "compat WalkTo did not retain the request-level denial"
    );
    let seat = assert_seat_idle(&fixture);
    assert_eq!(
        fixture.play.script_state(&fixture.name),
        script::RunState::Running,
        "one denied WalkTo must not fail its Load owner"
    );
    let (nav_logs, repair_waits) = assert_no_repair_waits(&fixture.name);
    fixture.play.script_stop(&fixture.name);
    let receipt = json!({
        "cell": "A2-compat-mort-myre-denied",
        "account": fixture.name,
        "server_stage": {"priestperil": 61, "druidspirit": 0},
        "wrong_memory": {"druidspirit": 10, "run": run_receipt(expected_run)},
        "global_allow_quest_start": true,
        "request_allow_quest_start": false,
        "server_refusal_fact": format!("{fact:?}"),
        "compat_result": result,
        "outer_terminal_logs": logs,
        "seat_idle": seat,
        "navigation_logs": nav_logs,
        "repair_waits": repair_waits,
        "session_only": true,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&receipt).expect("serialize A2 receipt")
    );
    println!("prereq cell outcome=completed test=live_compat_prereq_mort_myre_denied");
}

#[derive(Debug, Clone, serde::Serialize)]
struct ChatWitness {
    tick: u32,
    modal: i32,
    sequence: i32,
    text: String,
}

fn first_chat_witness(snapshot: &GameSnapshot) -> Option<ChatWitness> {
    if snapshot.modals().chat < 0 {
        return None;
    }
    let line = snapshot.chat_lines().first()?;
    let text = snapshot.chat_modal_texts().join("\n");
    (!text.is_empty()).then_some(ChatWitness {
        tick: snapshot.tick(),
        modal: snapshot.modals().chat,
        sequence: line.sequence,
        text,
    })
}

/// Waits that ended on their content signal. A Talk the engine cancelled
/// ("I can't reach that!", `end=unreachable`) was sent again and is not a
/// completed wait.
fn completed_wait_count(logs: &[String], wait: &str) -> usize {
    let prefix = format!("prereq wait={wait} ");
    logs.iter()
        .filter(|line| line.contains(prefix.as_str()) && !line.contains("end=unreachable"))
        .count()
}

fn assert_wait_completed(logs: &[String], wait: &str) {
    assert!(
        completed_wait_count(logs, wait) > 0,
        "missing observed {wait} completion: {logs:?}"
    );
}

fn print_prereq_receipt(test: &str, receipt: &serde_json::Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(receipt).expect("serialize prerequisite receipt")
    );
    println!("prereq cell outcome=completed test={test}");
}

fn observed_quest_stage(fixture: &LiveFixture, quest: &str, stage: i32) -> serde_json::Value {
    let varp = quest_varp(&fixture.quests, quest);
    fixture
        .play
        .cheat(&fixture.name, &format!("getvar {varp}"))
        .expect("queue authoritative quest stage query");
    let status = wait_for(
        &fixture.play,
        &fixture.name,
        "observed authoritative quest stage",
        Duration::from_secs(30),
        |play| {
            play.statuses().into_iter().find(|row| {
                row.username == fixture.name
                    && row.chat_head.contains(&format!("get {varp}: {stage}"))
            })
        },
    );
    json!({
        "quest": quest,
        "varp": varp,
        "stage": stage,
        "server_chat_head": status.chat_head,
    })
}

fn drezel_advice_page_matches(texts: &[String]) -> bool {
    api::snapshot::chat_page_matches(texts, "I cannot let you pass without warning you.")
}

#[test]
fn drezel_advice_match_uses_rendered_widget_boundaries() {
    let warning = [
        "Ah, adventurer. For all the assistance you",
        "have given both myself and Misthalin in",
        "your actions, I cannot let you pass without",
        "warning you.",
    ]
    .map(str::to_owned);
    assert!(drezel_advice_page_matches(&warning));
    assert!(!drezel_advice_page_matches(&[
        "Here, take it again, but please try and be more careful this time.".to_owned()
    ]));
}

fn wait_for_drezel_advice_page(fixture: &LiveFixture) -> (ChatWitness, String) {
    wait_for(
        &fixture.play,
        &fixture.name,
        "observed Drezel advice page",
        Duration::from_secs(240),
        |_| {
            let logs = slot_nav_logs(&fixture.name);
            let w5 = logs
                .iter()
                .position(|line| line.contains("prereq wait=W5 "))
                .or_else(|| {
                    logs.iter()
                        .position(|line| line.contains("prereq wait-start=W5 "))
                })?;
            let w3 = logs
                .iter()
                .skip(w5 + 1)
                .find(|line| line.contains("prereq wait=W3 "))
                .cloned()?;
            let snapshot = fixture.snapshot.lock().expect("game snapshot mutex");
            let page = first_chat_witness(&snapshot)?;
            if !drezel_advice_page_matches(snapshot.chat_modal_texts()) {
                return None;
            }
            Some((page, w3))
        },
    )
}

fn wait_for_live_approach(fixture: &LiveFixture) -> (u64, serde_json::Value) {
    wait_for(
        &fixture.play,
        &fixture.name,
        "compat route approaching without a modal",
        Duration::from_secs(240),
        |play| {
            let request_id = play.script_live_walk_request_id(&fixture.name)?;
            let inner_request_id = play.script_live_follow_request_id(&fixture.name)?;
            if request_id == inner_request_id {
                return None;
            }
            let row = play
                .statuses()
                .into_iter()
                .find(|row| row.username == fixture.name && row.ingame && row.scene_state == 2)?;
            let snapshot = fixture.snapshot.lock().expect("game snapshot mutex");
            if snapshot.modals().chat >= 0 {
                return None;
            }
            Some((
                request_id,
                json!({
                    "tick": snapshot.tick(),
                    "tile": [row.tile_x, row.tile_z, row.tile_level],
                    "chat_modal": snapshot.modals().chat,
                    "inner_request_id": inner_request_id,
                }),
            ))
        },
    )
}

fn wait_for_chat_steady(fixture: &LiveFixture) -> (ChatWitness, ChatWitness) {
    // A Continue admitted before AbortWalk can still return its page.
    // The trace oracle rejects every page action after the accepted AbortWalk.
    let mut baseline = None::<ChatWitness>;
    wait_for_snapshot(
        fixture,
        "chat page remains unclicked after AbortWalk",
        Duration::from_secs(30),
        |snapshot| {
            let Some(current) = first_chat_witness(snapshot) else {
                baseline = None;
                return None;
            };
            if let Some(before) = baseline.as_ref() {
                if current.modal == before.modal
                    && current.sequence == before.sequence
                    && current.text == before.text
                {
                    return (snapshot.tick() >= before.tick.saturating_add(2))
                        .then(|| (before.clone(), current));
                }
            }
            baseline = Some(current);
            None
        },
    )
}

fn gift_pie_counts(fixture: &LiveFixture, snapshot: &GameSnapshot) -> (i32, i32) {
    let meat = item_id(&fixture.selected, "meat_pie");
    let apple = item_id(&fixture.selected, "apple_pie");
    (
        item_count(snapshot.inventory(), meat),
        item_count(snapshot.inventory(), apple),
    )
}

fn bank_pie_counts(fixture: &LiveFixture, snapshot: &GameSnapshot) -> (i32, i32) {
    let meat = item_id(&fixture.selected, "meat_pie");
    let apple = item_id(&fixture.selected, "apple_pie");
    (
        item_count(snapshot.bank(), meat),
        item_count(snapshot.bank(), apple),
    )
}

fn start_default_gatherer(fixture: &LiveFixture) {
    fixture
        .play
        .script_start(
            &fixture.name,
            script::CompiledId("Gatherer"),
            serde_json::Map::new(),
        )
        .expect("start default Gatherer bank-trip card");
}

fn live_prereq_fixture(pack_slots: usize, full_bank: bool) -> (LiveFixture, Vec<String>) {
    let selected =
        api::game_data::for_revision(ClientRevision::R289).expect("selected R289 game data");
    let pack_aliases = inventory_seed_aliases(&selected, pack_slots);
    let mut seed_commands = item_seed_commands("give", &pack_aliases);
    if full_bank {
        seed_commands.extend(full_bank_seed_commands(&selected, &pack_aliases));
    } else {
        seed_commands.push("givebank dagger_wolfbane 1".to_owned());
    }
    let fixture = make_fixture_with_seed_commands(
        PREREQ_EAST,
        &[("priestperil", 61), ("druidspirit", 0)],
        true,
        true,
        &seed_commands,
    );
    (fixture, pack_aliases)
}

#[test]
#[ignore = "requires LIVE=1 and the explicitly configured local Engine L on 47594/3080"]
fn live_manual_prereq_holy_barrier_allowed() {
    if !api::test_support::game_data_available() {
        return;
    }
    let fixture = make_fixture(
        PREREQ_WEST,
        &[("priestperil", 60), ("druidspirit", 0)],
        true,
        true,
    );
    let previous_generation = lifecycle_generation(&fixture);
    let route = manual_map_walk(&fixture, PREREQ_EAST);
    let run = wait_for_manual_walk_run(&fixture);
    let fact = refusal_fact(&fixture, "priestperil", Duration::from_secs(240));
    assert_holy_barrier_refusal_fact(&fact);
    assert_eq!(
        fact.observed.run, run,
        "GateRefused belongs to this manual WalkTo"
    );
    let terminal = wait_for_manual_terminal(
        &fixture,
        run,
        previous_generation,
        script::ScriptTerminalState::Completed,
        Duration::from_secs(600),
    );
    let destination = wait_for_tile(&fixture, PREREQ_EAST);
    let logs = slot_nav_logs(&fixture.name);
    for wait in ["W3", "W4", "W5"] {
        assert_wait_completed(&logs, wait);
    }
    assert!(
        !logs
            .iter()
            .any(|line| line.contains("WaitSpent") || line.contains("MachineEnded")),
        "allowed holy-barrier path ended on a wait or machine limit: {logs:?}"
    );
    let receipt = json!({
        "cell": "A1-manual-holy-barrier-allowed",
        "account": fixture.name,
        "server_stage": {"priestperil": 60, "druidspirit": 0},
        "global_allow_prereq_dialogue": true,
        "manual_route": route,
        "server_refusal_fact": format!("{fact:?}"),
        "manual_terminal": terminal,
        "destination": destination,
        "navigation_logs": logs,
        "session_only": true,
    });
    print_prereq_receipt("live_manual_prereq_holy_barrier_allowed", &receipt);
}

#[test]
#[ignore = "requires LIVE=1 and the explicitly configured local Engine L on 47594/3080"]
fn live_manual_prereq_holy_barrier_wrong_memory_retries() {
    if !api::test_support::game_data_available() {
        return;
    }
    let fixture = make_fixture_with_seed_commands(
        PREREQ_WEST,
        &[("priestperil", 60), ("druidspirit", 0)],
        true,
        true,
        &["givebank dagger_wolfbane 1".to_owned()],
    );
    seed_wrong_memory(&fixture, "priestperil", 61, fixture.seed_run);
    let previous_generation = lifecycle_generation(&fixture);
    let route = manual_map_walk(&fixture, PREREQ_EAST);
    assert_eq!(
        route["admission"]["kind"], "walking",
        "wrong same-run memory must attempt the direct crossing before repair"
    );
    let fact = refusal_fact(&fixture, "priestperil", Duration::from_secs(240));
    assert_holy_barrier_refusal_fact(&fact);
    assert_eq!(
        fact.observed.run, fixture.seed_run,
        "direct GateRefused must keep the run that witnessed its page",
    );
    let run = wait_for_manual_walk_run(&fixture);
    assert_ne!(
        fact.observed.run, run,
        "hidden card must keep a fresh native identity"
    );
    let terminal = wait_for_manual_terminal(
        &fixture,
        run,
        previous_generation,
        script::ScriptTerminalState::Completed,
        Duration::from_secs(600),
    );
    let destination = wait_for_tile(&fixture, PREREQ_EAST);
    let logs = slot_nav_logs(&fixture.name);
    for wait in ["W3", "W4", "W5"] {
        assert_wait_completed(&logs, wait);
    }
    assert_eq!(
        completed_wait_count(&logs, "W5"),
        1,
        "wrong-memory repair talked twice"
    );
    assert_refusal_record_precedes_handover(&logs);
    let receipt = json!({
        "cell": "A1-manual-holy-barrier-wrong-memory-retries",
        "account": fixture.name,
        "server_stage": {"priestperil": 60, "druidspirit": 0},
        "wrong_memory": {"priestperil": 61, "run": run_receipt(fixture.seed_run)},
        "manual_route": route,
        "server_refusal_fact": format!("{fact:?}"),
        "manual_terminal": terminal,
        "destination": destination,
        "navigation_logs": logs,
        "session_only": true,
    });
    print_prereq_receipt(
        "live_manual_prereq_holy_barrier_wrong_memory_retries",
        &receipt,
    );
}

#[test]
#[ignore = "requires LIVE=1 and the explicitly configured local Engine L on 47594/3080"]
fn live_compat_prereq_holy_barrier_allowed() {
    if !api::test_support::game_data_available() {
        return;
    }
    let fixture = make_fixture(
        PREREQ_WEST,
        &[("priestperil", 60), ("druidspirit", 0)],
        true,
        true,
    );
    let run = start_compat(
        &fixture,
        compat_holy_source(
            PREREQ_EAST,
            "allowPrereqDialogue: true, allowQuestStart: true",
            false,
        ),
    );
    let fact = refusal_fact(&fixture, "priestperil", Duration::from_secs(240));
    assert_holy_barrier_refusal_fact(&fact);
    assert_eq!(
        fact.observed.run, run,
        "GateRefused belongs to the compat run"
    );
    let (result, logs) = wait_compat_result(&fixture, Duration::from_secs(600));
    assert_eq!(
        result["terminal_count"], 1,
        "expected one outer WalkTo terminal"
    );
    assert_eq!(
        result["first"].as_bool(),
        Some(true),
        "compat WalkTo did not arrive"
    );
    let destination = wait_for_tile(&fixture, PREREQ_EAST);
    let seat = assert_seat_idle(&fixture);
    for wait in ["W3", "W4", "W5"] {
        assert_wait_completed(&slot_nav_logs(&fixture.name), wait);
    }
    let navigation_logs = slot_nav_logs(&fixture.name);
    let receipt = json!({
        "cell": "A1-compat-holy-barrier-allowed",
        "account": fixture.name,
        "run": run_receipt(run),
        "server_stage": {"priestperil": 60, "druidspirit": 0},
        "request_allow_prereq_dialogue": true,
        "request_allow_quest_start": true,
        "server_refusal_fact": format!("{fact:?}"),
        "compat_result": result,
        "outer_terminal_logs": logs,
        "seat_idle": seat,
        "destination": destination,
        "navigation_logs": navigation_logs,
        "session_only": true,
    });
    stop_compat(&fixture);
    print_prereq_receipt("live_compat_prereq_holy_barrier_allowed", &receipt);
}

#[test]
#[ignore = "requires LIVE=1 and the explicitly configured local Engine L on 47594/3080"]
fn live_compat_prereq_holy_barrier_global_on_script_inherit() {
    if !api::test_support::game_data_available() {
        return;
    }
    let fixture = make_fixture(
        PREREQ_WEST,
        &[("priestperil", 60), ("druidspirit", 0)],
        true,
        true,
    );
    let script_permission = script::native::WalkPermissions::default();
    assert!(
        !script_permission.allow_prereq_dialogue,
        "compat script grant must be off for the Inherit case"
    );
    let run = start_compat(&fixture, compat_holy_source(PREREQ_EAST, "", false));
    let fact = refusal_fact(&fixture, "priestperil", Duration::from_secs(240));
    assert_holy_barrier_refusal_fact(&fact);
    assert_eq!(
        fact.observed.run, run,
        "GateRefused belongs to the compat run"
    );
    let (result, logs) = wait_compat_result(&fixture, Duration::from_secs(600));
    assert_eq!(
        result["terminal_count"], 1,
        "expected one outer WalkTo terminal"
    );
    assert_eq!(
        result["first"].as_bool(),
        Some(true),
        "Inherit under the global grant was treated as NotAllowed"
    );
    let destination = wait_for_tile(&fixture, PREREQ_EAST);
    let seat = assert_seat_idle(&fixture);
    let navigation_logs = slot_nav_logs(&fixture.name);
    let receipt = json!({
        "cell": "A1-compat-global-on-script-inherit",
        "account": fixture.name,
        "run": run_receipt(run),
        "server_stage": {"priestperil": 60, "druidspirit": 0},
        "global_allow_prereq_dialogue": true,
        "script_allow_prereq_dialogue": script_permission.allow_prereq_dialogue,
        "request_allow_prereq_dialogue": "Inherit (omitted)",
        "server_refusal_fact": format!("{fact:?}"),
        "compat_result": result,
        "outer_terminal_logs": logs,
        "seat_idle": seat,
        "destination": destination,
        "navigation_logs": navigation_logs,
        "session_only": true,
    });
    stop_compat(&fixture);
    print_prereq_receipt(
        "live_compat_prereq_holy_barrier_global_on_script_inherit",
        &receipt,
    );
}

#[test]
#[ignore = "requires LIVE=1 and the explicitly configured local Engine L on 47594/3080"]
fn live_compat_prereq_holy_barrier_abort_approach() {
    if !api::test_support::game_data_available() {
        return;
    }
    let fixture = make_fixture(
        PREREQ_WEST,
        &[("priestperil", 60), ("druidspirit", 0)],
        true,
        true,
    );
    let run = start_compat(
        &fixture,
        compat_holy_source(
            PREREQ_EAST,
            "allowPrereqDialogue: true, allowQuestStart: true",
            false,
        ),
    );
    let fact = refusal_fact(&fixture, "priestperil", Duration::from_secs(240));
    assert_holy_barrier_refusal_fact(&fact);
    assert_eq!(
        fact.observed.run, run,
        "GateRefused belongs to the compat run"
    );
    let (request_id, approach) = wait_for_live_approach(&fixture);
    fixture
        .play
        .script_api_live_abort_walk(&fixture.name, request_id)
        .expect("dispatch AbortWalk for the exact live outer id");
    let (result, terminal_logs) = wait_compat_result(&fixture, Duration::from_secs(240));
    assert_eq!(
        result["terminal_count"], 1,
        "expected one outer WalkTo terminal"
    );
    assert_eq!(
        result["first"].as_bool(),
        Some(false),
        "AbortWalk was not observed"
    );
    assert_seat_idle(&fixture);
    let navigation_logs = slot_nav_logs(&fixture.name);
    assert!(
        !navigation_logs
            .iter()
            .any(|line| line.contains("prereq wait-start=W5 ")),
        "compat route opened a Drezel Talk after approach AbortWalk: {navigation_logs:?}"
    );
    let snapshot = fixture.snapshot.lock().expect("game snapshot mutex");
    assert!(
        snapshot.modals().chat < 0,
        "chat modal opened after approach AbortWalk"
    );
    let receipt = json!({
        "cell": "A1-compat-abort-approach",
        "account": fixture.name,
        "run": run_receipt(run),
        "outer_request_id": request_id,
        "approach": approach,
        "server_refusal_fact": format!("{fact:?}"),
        "compat_result": result,
        "outer_terminal_logs": terminal_logs,
        "navigation_logs": navigation_logs,
        "chat_modal_after_abort": snapshot.modals().chat,
        "session_only": true,
    });
    drop(snapshot);
    stop_compat(&fixture);
    print_prereq_receipt("live_compat_prereq_holy_barrier_abort_approach", &receipt);
}

#[test]
#[ignore = "requires LIVE=1 and the explicitly configured local Engine L on 47594/3080"]
fn live_compat_prereq_holy_barrier_abort_advice_page() {
    if !api::test_support::game_data_available() {
        return;
    }
    let fixture = make_fixture_with_seed_commands(
        PREREQ_WEST,
        &[("priestperil", 60), ("druidspirit", 0)],
        true,
        true,
        &["givebank dagger_wolfbane 1".to_owned()],
    );
    let run = start_compat(
        &fixture,
        compat_holy_source(
            PREREQ_EAST,
            "allowPrereqDialogue: true, allowQuestStart: true",
            false,
        ),
    );
    let fact = refusal_fact(&fixture, "priestperil", Duration::from_secs(240));
    assert_holy_barrier_refusal_fact(&fact);
    assert_eq!(
        fact.observed.run, run,
        "GateRefused belongs to the compat run"
    );
    let (page, w3_log) = wait_for_drezel_advice_page(&fixture);
    assert!(
        w3_log.contains("since=")
            && w3_log.contains("at=")
            && w3_log.contains("count=")
            && w3_log.contains("bound=")
            && w3_log.contains("end="),
        "advice page lacks the observed W3 evidence record: {w3_log}"
    );
    assert!(page.modal >= 0, "advice page is not open: {page:?}");
    let request_id = fixture
        .play
        .script_live_walk_request_id(&fixture.name)
        .expect("advice page must retain the outer walk id");
    fixture
        .play
        .script_api_live_abort_walk(&fixture.name, request_id)
        .expect("dispatch AbortWalk for the exact live outer id");
    let (result, terminal_logs) = wait_compat_result(&fixture, Duration::from_secs(240));
    assert_eq!(
        result["terminal_count"], 1,
        "expected one outer WalkTo terminal"
    );
    assert_eq!(
        result["first"].as_bool(),
        Some(false),
        "AbortWalk was not observed"
    );
    let seat = assert_seat_idle(&fixture);
    let logs_at_abort_terminal = slot_nav_logs(&fixture.name);
    let (settled_page, steady_page) = wait_for_chat_steady(&fixture);
    let navigation_logs = slot_nav_logs(&fixture.name);
    let later_w3_starts = navigation_logs
        .iter()
        .skip(logs_at_abort_terminal.len())
        .filter(|line| line.contains("prereq wait-start=W3 "))
        .count();
    assert_eq!(
        later_w3_starts, 0,
        "host started another dialogue page after AbortWalk: {navigation_logs:?}"
    );
    let receipt = json!({
        "cell": "A1-compat-abort-advice-page",
        "account": fixture.name,
        "run": run_receipt(run),
        "outer_request_id": request_id,
        "server_refusal_fact": format!("{fact:?}"),
        "advice_page": page,
        "advice_wait_observed": true,
        "compat_result": result,
        "outer_terminal_logs": terminal_logs,
        "seat_idle": seat,
        "settled_page_after_abort": settled_page,
        "same_page_after_two_observed_ticks": steady_page,
        "later_w3_starts": later_w3_starts,
        "navigation_log_count_at_abort_terminal": logs_at_abort_terminal.len(),
        "navigation_logs": navigation_logs,
        "session_only": true,
    });
    stop_compat(&fixture);
    print_prereq_receipt(
        "live_compat_prereq_holy_barrier_abort_advice_page",
        &receipt,
    );
}

#[test]
#[ignore = "requires LIVE=1 and the explicitly configured local Engine L on 47594/3080"]
fn live_compat_prereq_holy_barrier_denied_second_walk_same_run() {
    if !api::test_support::game_data_available() {
        return;
    }
    let fixture = make_fixture(
        PREREQ_WEST,
        &[("priestperil", 60), ("druidspirit", 0)],
        false,
        true,
    );
    let expected_run = fixture.seed_run.next_run().expect("next compat run key");
    seed_wrong_memory(&fixture, "priestperil", 61, expected_run);
    let run = start_compat(&fixture, compat_holy_source(PREREQ_EAST, "", true));
    assert_eq!(
        run, expected_run,
        "compat run differs from seeded memory run"
    );
    let fact = refusal_fact(&fixture, "priestperil", Duration::from_secs(240));
    assert_holy_barrier_refusal_fact(&fact);
    assert_eq!(
        fact.observed.run, run,
        "GateRefused belongs to the compat run"
    );
    let (result, logs) = wait_compat_result(&fixture, Duration::from_secs(600));
    assert_eq!(
        result["terminal_count"], 2,
        "expected two same-run WalkTo terminals"
    );
    assert_eq!(
        result["first"].as_bool(),
        Some(false),
        "first WalkTo was not denied"
    );
    assert_eq!(
        result["second"].as_bool(),
        Some(false),
        "second WalkTo was not denied"
    );
    let (navigation_logs, repair_waits) = assert_no_repair_waits(&fixture.name);
    let seat = assert_seat_idle(&fixture);
    let receipt = json!({
        "cell": "A1-compat-denied-second-walk-same-run",
        "account": fixture.name,
        "run": run_receipt(run),
        "server_stage": {"priestperil": 60, "druidspirit": 0},
        "global_allow_prereq_dialogue": false,
        "wrong_memory": {"priestperil": 61, "run": run_receipt(expected_run)},
        "server_refusal_fact": format!("{fact:?}"),
        "same_run_compat_result": result,
        "outer_terminal_logs": logs,
        "navigation_logs": navigation_logs,
        "repair_waits": repair_waits,
        "seat_idle": seat,
        "session_only": true,
    });
    stop_compat(&fixture);
    print_prereq_receipt(
        "live_compat_prereq_holy_barrier_denied_second_walk_same_run",
        &receipt,
    );
}

fn run_mort_myre_gift(full_bank: bool, run_gatherer: bool) {
    let (fixture, pack_aliases) = live_prereq_fixture(22, full_bank);
    let seeded_inventory_tick = wait_for_inventory_slots(&fixture, 22);
    let script_allow_quest_start = script::native::WalkPermissions::default().allow_quest_start;
    assert!(
        !script_allow_quest_start,
        "compat script grant must be off for the Mort Myre Inherit case"
    );
    let run = start_compat(&fixture, compat_holy_source(PREREQ_SWAMP, "", false));
    let fact = wait_for(
        &fixture.play,
        &fixture.name,
        "observed Mort Myre start item-box fact",
        Duration::from_secs(240),
        |_| {
            fixture
                .state
                .prereq_snapshot()
                .facts()
                .iter()
                .find(|fact| {
                    fact.quest.0.as_ref() == "druidspirit"
                        && fact.possible.min == Some(5)
                        && fact.possible.max == Some(5)
                        && matches!(&fact.source, FactSource::Page { .. })
                })
                .cloned()
        },
    );
    assert_eq!(
        fact.observed.run, run,
        "Mort Myre item-box fact belongs to compat run"
    );
    assert!(
        fact.refused.is_none(),
        "known-false Mort Myre start must not probe"
    );
    let (gift_tick, gift_counts, gift_chat) = wait_for_snapshot(
        &fixture,
        "observed Drezel's three meat pies and three apple pies",
        Duration::from_secs(600),
        |snapshot| {
            let counts = gift_pie_counts(&fixture, snapshot);
            (counts == (3, 3) && snapshot.inventory().len() == 28).then_some((
                snapshot.tick(),
                counts,
                snapshot
                    .chat_lines()
                    .iter()
                    .map(|line| line.text.clone())
                    .collect::<Vec<_>>(),
            ))
        },
    );
    let stage = observed_quest_stage(&fixture, "druidspirit", 5);
    let (walk_result, walk_logs) = wait_compat_result(&fixture, Duration::from_secs(600));
    assert_eq!(
        walk_result["terminal_count"], 1,
        "expected one outer WalkTo terminal"
    );
    assert_eq!(
        walk_result["first"].as_bool(),
        Some(true),
        "Mort Myre start WalkTo did not arrive"
    );
    let destination = wait_for_tile(&fixture, PREREQ_SWAMP);
    let seat = assert_seat_idle(&fixture);
    let navigation_logs = slot_nav_logs(&fixture.name);
    for wait in ["W3", "W4", "W5"] {
        assert_wait_completed(&navigation_logs, wait);
    }
    let receipt = json!({
        "cell": "A2-compat-mort-myre-started",
        "account": fixture.name,
        "run": run_receipt(run),
        "server_start_stage": {"priestperil": 61, "druidspirit": 0},
        "script_allow_quest_start": script_allow_quest_start,
        "seeded_inventory_tick": seeded_inventory_tick,
        "seeded_pack_aliases": pack_aliases,
        "start_item_box_fact": format!("{fact:?}"),
        "gift_tick": gift_tick,
        "inventory_pies": {"meat_pie": gift_counts.0, "apple_pie": gift_counts.1},
        "gift_chat_lines": gift_chat,
        "authoritative_stage": stage,
        "compat_result": walk_result,
        "outer_terminal_logs": walk_logs,
        "seat_idle": seat,
        "destination": destination,
        "navigation_logs": navigation_logs,
        "session_only": true,
    });
    stop_compat(&fixture);
    if !run_gatherer {
        print_prereq_receipt("live_compat_prereq_mort_myre_started_and_gift", &receipt);
        return;
    }
    let mut start_receipt = receipt.clone();
    assert!(
        start_receipt
            .as_object_mut()
            .expect("Mort Myre start receipt object")
            .remove("navigation_logs")
            .is_some(),
        "remove nested navigation logs so wait evidence is emitted exactly once"
    );

    let gatherer_receipt = if full_bank {
        let expected_pies = gift_pie_counts(
            &fixture,
            &fixture.snapshot.lock().expect("game snapshot mutex"),
        );
        assert_eq!(
            expected_pies,
            (3, 3),
            "full-bank case must carry the six pies"
        );
        start_default_gatherer(&fixture);
        let (failure, bank_receipt) = wait_for(
            &fixture.play,
            &fixture.name,
            "typed full-bank Gatherer refusal",
            Duration::from_secs(900),
            |play| {
                let status = play.script_native_status(&fixture.name)?;
                let failure = status.failure.as_ref()?;
                if status.card != script::CompiledId("Gatherer")
                    || failure.code.as_ref() != "bank-deposit-failed"
                    || failure.message.as_ref() != "bank deposit refused: bank is full"
                {
                    return None;
                }
                let snapshot = fixture.snapshot.lock().expect("game snapshot mutex");
                if !snapshot.bank_loaded()
                    || snapshot.bank_size() as usize != FULL_BANK_SLOTS
                    || snapshot.bank().len() != FULL_BANK_SLOTS
                {
                    return None;
                }
                let unique_ids = snapshot
                    .bank()
                    .iter()
                    .map(|item| item.def.id)
                    .collect::<HashSet<_>>();
                if unique_ids.len() != FULL_BANK_SLOTS
                    || gift_pie_counts(&fixture, &snapshot) != (3, 3)
                {
                    return None;
                }
                Some((
                    (*failure).clone(),
                    json!({
                        "tick": snapshot.tick(),
                        "bank_size": snapshot.bank_size(),
                        "bank_rows": snapshot.bank().len(),
                        "unique_bank_ids": unique_ids.len(),
                        "inventory_pies": gift_pie_counts(&fixture, &snapshot),
                        "bank_pies": bank_pie_counts(&fixture, &snapshot),
                    }),
                ))
            },
        );
        let mut retained_ids = pack_aliases
            .iter()
            .map(|alias| item_id(&fixture.selected, alias))
            .collect::<HashSet<_>>();
        retained_ids.insert(item_id(&fixture.selected, "meat_pie"));
        retained_ids.insert(item_id(&fixture.selected, "apple_pie"));
        let snapshot = fixture.snapshot.lock().expect("game snapshot mutex");
        let dropped = snapshot
            .ground_items()
            .iter()
            .filter(|item| retained_ids.contains(&item.def.id))
            .map(|item| json!({"id": item.def.id, "count": item.count, "tile": item.tile}))
            .collect::<Vec<_>>();
        assert!(
            dropped.is_empty(),
            "full-bank Gatherer dropped pack items: {dropped:?}"
        );
        assert_eq!(gift_pie_counts(&fixture, &snapshot), (3, 3));
        let receipt = json!({
            "mort_myre_start": start_receipt,
            "gatherer_failure": {
                "code": failure.code.to_string(),
                "message": failure.message.to_string(),
            },
            "full_bank": bank_receipt,
            "dropped_seed_items": dropped,
            "navigation_logs": slot_nav_logs(&fixture.name),
        });
        drop(snapshot);
        fixture.play.script_stop(&fixture.name);
        wait_for(
            &fixture.play,
            &fixture.name,
            "full-bank Gatherer stop",
            Duration::from_secs(30),
            |play| (play.script_state(&fixture.name) == script::RunState::Idle).then_some(()),
        );
        receipt
    } else {
        start_default_gatherer(&fixture);
        let bank_receipt = wait_for(
            &fixture.play,
            &fixture.name,
            "normal Gatherer deposit of incidental pies",
            Duration::from_secs(900),
            |play| {
                let status = play.script_native_status(&fixture.name)?;
                if status.card != script::CompiledId("Gatherer") {
                    return None;
                }
                let snapshot = fixture.snapshot.lock().expect("game snapshot mutex");
                let inventory_pies = gift_pie_counts(&fixture, &snapshot);
                let bank_pies = bank_pie_counts(&fixture, &snapshot);
                (snapshot.bank_loaded() && inventory_pies == (0, 0) && bank_pies == (3, 3))
                    .then_some(json!({
                        "tick": snapshot.tick(),
                        "bank_rows": snapshot.bank().len(),
                        "inventory_pies": inventory_pies,
                        "bank_pies": bank_pies,
                        "phase": format!("{:?}", status.phase),
                    }))
            },
        );
        let mut retained_ids = pack_aliases
            .iter()
            .map(|alias| item_id(&fixture.selected, alias))
            .collect::<HashSet<_>>();
        retained_ids.insert(item_id(&fixture.selected, "meat_pie"));
        retained_ids.insert(item_id(&fixture.selected, "apple_pie"));
        let snapshot = fixture.snapshot.lock().expect("game snapshot mutex");
        let dropped = snapshot
            .ground_items()
            .iter()
            .filter(|item| retained_ids.contains(&item.def.id))
            .map(|item| json!({"id": item.def.id, "count": item.count, "tile": item.tile}))
            .collect::<Vec<_>>();
        assert!(
            dropped.is_empty(),
            "room-bank Gatherer dropped pack items: {dropped:?}"
        );
        let receipt = json!({
            "mort_myre_start": start_receipt,
            "gatherer_bank_deposit": bank_receipt,
            "dropped_seed_items": dropped,
            "navigation_logs": slot_nav_logs(&fixture.name),
        });
        drop(snapshot);
        fixture.play.script_stop(&fixture.name);
        wait_for(
            &fixture.play,
            &fixture.name,
            "room-bank Gatherer stop",
            Duration::from_secs(30),
            |play| (play.script_state(&fixture.name) == script::RunState::Idle).then_some(()),
        );
        receipt
    };
    print_prereq_receipt(
        if full_bank {
            "live_gatherer_prereq_mort_myre_gift_full_bank"
        } else {
            "live_gatherer_prereq_mort_myre_gift_room_bank"
        },
        &gatherer_receipt,
    );
}

#[test]
#[ignore = "requires LIVE=1 and the explicitly configured local Engine L on 47594/3080"]
fn live_compat_prereq_mort_myre_started_and_gift() {
    if !api::test_support::game_data_available() {
        return;
    }
    run_mort_myre_gift(false, false);
}

#[test]
#[ignore = "requires LIVE=1 and the explicitly configured local Engine L on 47594/3080"]
fn live_gatherer_prereq_mort_myre_gift_room_bank() {
    if !api::test_support::game_data_available() {
        return;
    }
    run_mort_myre_gift(false, true);
}

#[test]
#[ignore = "requires LIVE=1 and the explicitly configured local Engine L on 47594/3080"]
fn live_gatherer_prereq_mort_myre_gift_full_bank() {
    if !api::test_support::game_data_available() {
        return;
    }
    run_mort_myre_gift(true, true);
}
