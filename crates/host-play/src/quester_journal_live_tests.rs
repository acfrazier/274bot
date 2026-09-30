//! Headless LIVE proofs for Quester journal evidence and the released Cook card.
//!
//! The synthetic card deliberately uses the Rune Mysteries journal branches from
//! the R289 content pack while retaining the checked Cook Path header.  It is a
//! fixture, not a second product Path: the test installs it through the
//! test-only [`ScriptStartHandle::start_test_script`] seam and exercises the
//! ordinary host pump.
//!
//! The tests are ignored because they need the shared local R289 engine.  Run
//! one at a time with a throwaway HOME, for example:
//!
//! ```text
//! LIVE=1 cargo test -p host-play --lib live_quester_journal_synthetic_runemysteries -- --ignored --nocapture --test-threads=1
//! LIVE=1 cargo test -p host-play --lib live_quester_cook_reaches_complete -- --ignored --nocapture --test-threads=1
//! ```

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::quest_facts::QuestCatalog;
use api::selected::{ClientRevision, FactKey, Truth};
use script::native::{NativePhase, ScriptStatus, StatusValue};
use script::quester::compile::{compile_uncached_for_test, decode_cook};
use script::quester::path::{
    PathDocument, PredicateDocument, ProgressColourDocument, ProgressDocument,
    ProgressFlagDocument, ProgressRuleDocument, SequenceDocument,
};
use script::quester::runner::Quester;
use vault::{Profile, ProfileSettings};

use super::{run_with_template, ProfileOptions, ScriptStartHandle, SharedClientTemplate};

const SYNTHETIC_QUEST: &str = "runemysteries";
const ACCOUNT_UID: i32 = 274_279_003;
const SYNTHETIC_TIMEOUT: Duration = Duration::from_secs(240);
const COOK_TIMEOUT: Duration = Duration::from_secs(1000);

#[derive(Clone, Copy)]
enum SetupMode {
    Synthetic,
    Cook,
}

#[derive(Default)]
struct SetupState {
    primed: bool,
    logout_after: Option<Instant>,
    logout_sent: bool,
    saw_offline: bool,
    relog_ready: bool,
}

/// The live engine is shared by the operator's tunnel.  Keep its profile and
/// cache paths explicit and never let a test resolve `~/.274bot`.
struct ThrowawayHome {
    path: PathBuf,
    previous: Option<String>,
}

impl ThrowawayHome {
    fn enter(label: &str) -> Self {
        let previous = std::env::var("HOME").ok();
        let path = std::env::temp_dir().join(format!(
            "274bot-quester-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).expect("create throwaway HOME");
        std::env::set_var("HOME", &path);
        Self { path, previous }
    }
}

impl Drop for ThrowawayHome {
    fn drop(&mut self) {
        match &self.previous {
            Some(previous) => std::env::set_var("HOME", previous),
            None => std::env::remove_var("HOME"),
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn live() -> bool {
    std::env::var("LIVE").as_deref() == Ok("1")
}

fn live_options(home: &Path) -> ProfileOptions {
    let port = std::env::var("BOT_GAME_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(45594);
    let http_port = std::env::var("BOT_HTTP_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(2080);
    let nav_pack = std::env::var_os("BOT_NAV_PACK")
        .map(PathBuf::from)
        .or_else(|| {
            let own = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/debug/nav/289/274bot.navpack");
            own.exists().then_some(own)
        });
    ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(port),
        http_port: Some(http_port),
        vault_path: Some(home.join("vault")),
        unpack_dir: Some(home.join("unpack")),
        nav_pack,
        engine_dir: std::env::var_os("BOT_ENGINE_DIR").map(PathBuf::from),
        ..ProfileOptions::default()
    }
}

fn account_name() -> String {
    format!(
        "qj{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_millis() % 1_000_000_000)
            .unwrap_or(0)
    )
}

fn profile(name: &str) -> Profile {
    Profile {
        username: name.to_string(),
        password: name.to_string().into(),
        uid: ACCOUNT_UID,
        settings: ProfileSettings::default(),
    }
}

fn prime(client: &mut client::client::Client, mode: SetupMode) {
    // Tutorial completion must be followed by a relog before the quest tab is
    // trusted.  The quest varp/item cheats only choose a deterministic fixture
    // state; all journal clicks and subsequent actions remain host-owned.
    interact::mainland_hop(client);
    match mode {
        SetupMode::Synthetic => {
            let _ = interact::cheat(client, "setvar runemysteries 3");
            let _ = interact::cheat(client, "give research_package 1");
        }
        SetupMode::Cook => {
            let _ = interact::cheat(client, "~clearinv");
            let _ = interact::cheat(client, "setvar cook 0");
        }
    }
}

fn post_relog(client: &mut client::client::Client, mode: SetupMode) {
    let (x, z) = match mode {
        // Aubury is the real Rune Mysteries branch-3 advance target.
        SetupMode::Synthetic => (3253, 3401),
        SetupMode::Cook => (3209, 3215),
    };
    assert_eq!(
        interact::cheat(client, &interact::tele_args(0, x, z)),
        client::CheatSend::Sent
    );
}

fn frame_hook(
    state: Arc<Mutex<SetupState>>,
    mode: SetupMode,
) -> impl Fn(&mut client::client::Client, &str, bool) + Send + Sync + 'static {
    move |client, _name, _hold| {
        let now = Instant::now();
        let mut state = state.lock().expect("quester live setup lock");
        if !client.ingame {
            if state.logout_sent {
                state.saw_offline = true;
            }
            return;
        }
        if client.scene_state != 2 {
            return;
        }
        if !state.primed {
            prime(client, mode);
            state.primed = true;
            state.logout_after = Some(now + Duration::from_secs(2));
            return;
        }
        if !state.logout_sent && state.logout_after.is_some_and(|deadline| now >= deadline) {
            let ifaces = Arc::clone(&client.ifaces);
            state.logout_sent = interact::logout(client, &ifaces);
            return;
        }
        if state.logout_sent && state.saw_offline && !state.relog_ready {
            post_relog(client, mode);
            state.relog_ready = true;
        }
    }
}

fn launch_live(
    mode: SetupMode,
    label: &str,
) -> (ThrowawayHome, super::Play, Arc<Mutex<SetupState>>, String) {
    let home = ThrowawayHome::enter(label);
    let options = live_options(&home.path);
    let server_profile = options
        .resolve(None)
        .expect("resolve local-289 profile")
        .bind()
        .expect("bind local-289 profile");
    let template =
        SharedClientTemplate::load(Arc::clone(&server_profile)).expect("load live template");
    let name = account_name();
    let state = Arc::new(Mutex::new(SetupState::default()));
    let hook = Arc::clone(&state);
    let play = run_with_template(
        template,
        false,
        vec![profile(name.as_str())],
        |_| (None, None),
        frame_hook(hook, mode),
    )
    .expect("start live Quester Play");
    (home, play, state, name)
}

fn wait_until(label: &str, timeout: Duration, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    loop {
        if ready() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{label} timed out after {timeout:?}"
        );
        thread::sleep(Duration::from_millis(100));
    }
}

fn wait_relogged(play: &super::Play, state: &Arc<Mutex<SetupState>>) {
    wait_until("live account relog", Duration::from_secs(90), || {
        state.lock().expect("setup state").relog_ready
            && play.statuses().iter().any(|status| {
                status.username.starts_with("qj") && status.ingame && status.scene_state == 2
            })
    });
}

fn field<'a>(status: &'a ScriptStatus, key: &str) -> &'a StatusValue {
    status
        .fields
        .iter()
        .find(|field| field.key == key)
        .map(|field| &field.value)
        .unwrap_or_else(|| panic!("status {key:?} missing from {status:?}"))
}

fn text<'a>(status: &'a ScriptStatus, key: &str) -> &'a str {
    match field(status, key) {
        StatusValue::Text(value) => value,
        other => panic!("status {key:?} is not Text: {other:?}"),
    }
}

fn truth(status: &ScriptStatus, key: &str) -> Truth {
    match field(status, key) {
        StatusValue::Truth(value) => *value,
        other => panic!("status {key:?} is not Truth: {other:?}"),
    }
}

fn wait_status(
    play: &super::Play,
    name: &str,
    timeout: Duration,
    label: &str,
    mut predicate: impl FnMut(&ScriptStatus) -> bool,
) -> Arc<ScriptStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = play.script_native_status(name) {
            if predicate(&status) {
                return status;
            }
            assert_ne!(
                status.phase,
                NativePhase::Blocked,
                "{label} blocked: {status:#?}"
            );
        }
        assert!(
            Instant::now() < deadline,
            "{label} timed out; status={:?}",
            play.script_native_status(name)
        );
        thread::sleep(Duration::from_millis(100));
    }
}

fn wait_test_start(handle: &ScriptStartHandle, name: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match handle.poll_start(name) {
            script::StartPoll::Pending => {}
            script::StartPoll::Settled(script::StartOutcome::Ready) => return,
            script::StartPoll::Settled(outcome) => panic!("Quester Start failed: {outcome:?}"),
            script::StartPoll::NotOwed => panic!("Quester Start outcome was lost"),
        }
        assert!(Instant::now() < deadline, "Quester Start did not settle");
        thread::sleep(Duration::from_millis(50));
    }
}

fn synthetic_document(no_match: bool) -> PathDocument {
    let mut document = decode_cook().expect("decode checked Cook Path");
    document.id = FactKey::new(SYNTHETIC_QUEST);
    document.display_name = "Synthetic Rune Mysteries journal".into();
    let role = &mut document.roles[0];
    role.progress_binding = FactKey::new("journal:runemysteries");

    let template = role.sequences[0].clone();
    let stages = ["rm:0", "rm:1", "rm:2", "rm:3", "rm:4", "rm:5", "rm:6"];
    role.sequences = stages
        .iter()
        .enumerate()
        .map(|(index, stage)| {
            let mut sequence = SequenceDocument {
                stage: FactKey::new(stage),
                required: Vec::new(),
                terminal: *stage == "rm:6",
                recovery_entry: None,
                steps: Vec::new(),
            };
            if !sequence.terminal {
                let mut step = template.steps[0].clone();
                step.id = FactKey::new(&format!("synthetic-rm-step-{index}"));
                step.advances = *stage == "rm:3";
                step.skip_if = PredicateDocument::Any(Vec::new());
                if *stage == "rm:3" {
                    // This is the real content branch: at varp 3 the journal
                    // asks for the Research Package at Aubury. The host
                    // dialogue machine performs the talk; it is not faked by
                    // changing the journal/status wire.
                    step.args = serde_json::json!({
                        "npc": "aubury",
                        "leash": 8,
                        "prefer": ["I have been sent here with a package for you."],
                    });
                    step.settle = PredicateDocument::Fact {
                        kind: "stage_in".into(),
                        version: 1,
                        args: serde_json::json!({
                            "quest": SYNTHETIC_QUEST,
                            "any": ["rm:4"],
                        }),
                    };
                } else {
                    // Keep every fixture branch other than the exercised
                    // Aubury branch passive. In particular, do not inherit a
                    // Cook Talk action that could emit unrelated wire input.
                    step.kind = "wait".into();
                    step.version = 1;
                    step.args = serde_json::json!({
                        "until": {
                            "Fact": {
                                "kind": "quest_colour",
                                "version": 1,
                                "args": {
                                    "quest": SYNTHETIC_QUEST,
                                    "is": "complete",
                                },
                            },
                        },
                        "max_ticks": 1000,
                    });
                    step.settle = PredicateDocument::Fact {
                        kind: "stage_in".into(),
                        version: 1,
                        args: serde_json::json!({
                            "quest": SYNTHETIC_QUEST,
                            "any": [format!("rm:{}", index + 1)],
                        }),
                    };
                }
                sequence.steps.push(step);
            }
            sequence
        })
        .collect();
    // These branches mirror content/scripts/quests/quest_runemysteries/scripts/
    // runemysteries_journal.rs2 (R289): varps 1..5 are the authored body
    // branches and the final else branch is the completed journal.

    let rules = if no_match {
        (1..=6)
            .rev()
            .map(|stage| ProgressRuleDocument {
                stage: FactKey::new(&format!("rm:{stage}")),
                all: vec!["fixture line that is never emitted".into()],
                any: Vec::new(),
                not: Vec::new(),
                varp: Some(stage),
            })
            .collect()
    } else {
        vec![
            ProgressRuleDocument {
                stage: FactKey::new("rm:6"),
                all: vec!["quest complete".into()],
                any: Vec::new(),
                not: Vec::new(),
                varp: Some(6),
            },
            ProgressRuleDocument {
                stage: FactKey::new("rm:5"),
                all: vec!["aubury was interested".into(), "research notes".into()],
                any: Vec::new(),
                not: Vec::new(),
                varp: Some(5),
            },
            ProgressRuleDocument {
                stage: FactKey::new("rm:4"),
                all: vec!["took the research package".into(), "delivered it".into()],
                any: Vec::new(),
                not: Vec::new(),
                varp: Some(4),
            },
            ProgressRuleDocument {
                stage: FactKey::new("rm:3"),
                all: vec!["research package".into(), "aubury".into()],
                any: Vec::new(),
                not: Vec::new(),
                varp: Some(3),
            },
            ProgressRuleDocument {
                stage: FactKey::new("rm:2"),
                all: vec!["gave the talisman".into(), "head wizard".into()],
                any: Vec::new(),
                not: Vec::new(),
                varp: Some(2),
            },
            ProgressRuleDocument {
                stage: FactKey::new("rm:1"),
                all: vec!["spoke to duke horacio".into(), "strange".into()],
                any: Vec::new(),
                not: Vec::new(),
                varp: Some(1),
            },
        ]
    };
    role.progress = Some(ProgressDocument {
        colour: ProgressColourDocument {
            not_started: FactKey::new("rm:0"),
            in_progress: FactKey::new("rm:1"),
            complete: FactKey::new("rm:6"),
        },
        rules,
        flags: vec![ProgressFlagDocument {
            flag: FactKey::new("rm:research-package"),
            all: vec!["research package".into()],
            any: Vec::new(),
            count: None,
        }],
        monotonic: false,
    });
    document
}

fn compile_synthetic(
    selected: &api::game_data::SelectedGameData,
    quests: &QuestCatalog,
    no_match: bool,
) -> Arc<script::quester::compile::CompiledPath> {
    let path = compile_uncached_for_test(&synthetic_document(no_match), selected, quests)
        .expect("compile synthetic Rune Mysteries Path");
    assert!(
        path.progress.rules.len() >= 3,
        "fixture must compile at least three progress rules"
    );
    println!(
        "synthetic compiled progress rules (no_match={no_match}): {}",
        path.progress.rules.len()
    );
    path
}

#[test]
#[ignore = "requires LIVE=1 and the shared tunnelled local R289 engine"]
fn live_quester_journal_synthetic_runemysteries() {
    assert!(
        live(),
        "live_quester_journal_synthetic_runemysteries requires LIVE=1"
    );
    let (home, play, setup, name) = launch_live(SetupMode::Synthetic, "journal");
    wait_relogged(&play, &setup);

    let selected = api::game_data::for_revision(ClientRevision::R289).expect("R289 game data");
    let quests =
        Arc::new(QuestCatalog::from_identity(selected.quest_identity()).expect("quest catalog"));
    let path = compile_synthetic(&selected, &quests, false);
    let no_match_path = compile_synthetic(&selected, &quests, true);
    let handle = play.script_start_handle();
    let run = handle
        .start_test_script(
            &name,
            Box::new(Quester::new(
                api::selected::RunKey {
                    slot: 0,
                    run: 0,
                    session: 0,
                },
                path,
                Arc::clone(&quests),
            )),
            Some(Arc::clone(&selected)),
        )
        .expect("install synthetic Quester");
    assert_eq!(
        run,
        play.script_native_run(&name).expect("synthetic run key")
    );
    play.wake(&name);

    let seeded = wait_status(
        &play,
        &name,
        SYNTHETIC_TIMEOUT,
        "seeded Rune Mysteries journal stage",
        |status| {
            status.phase == NativePhase::Working
                && text(status, "stage") == "rm:3"
                && text(status, "rule") == "rm:3"
                && text(status, "journal_lines").contains("Research Package")
                && truth(status, "needs_read") == Truth::False
        },
    );
    assert!(matches!(
        field(&seeded, "varp_hint"),
        StatusValue::Integer(3)
    ));
    match field(&seeded, "progress") {
        StatusValue::Quest(progress) => {
            assert!(
                progress.signals.is_empty(),
                "readable journal must not invent signals"
            );
        }
        other => panic!("progress status is not Quest: {other:?}"),
    }
    println!(
        "synthetic seeded journal status: run={:?} raw={:?} status={seeded:?}",
        seeded.run,
        text(&seeded, "journal_lines")
    );

    let advanced = wait_status(
        &play,
        &name,
        SYNTHETIC_TIMEOUT,
        "Rune Mysteries branch advance and fresh journal settle",
        |status| {
            status.phase == NativePhase::Working
                && text(status, "stage") == "rm:4"
                && text(status, "rule") == "rm:4"
                && text(status, "journal_lines").contains("delivered it")
                && truth(status, "needs_read") == Truth::False
        },
    );
    assert!(matches!(
        field(&advanced, "varp_hint"),
        StatusValue::Integer(4)
    ));
    match field(&advanced, "progress") {
        StatusValue::Quest(progress) => assert!(progress.signals.is_empty()),
        other => panic!("progress status is not Quest: {other:?}"),
    }
    println!(
        "synthetic advanced journal status: run={:?} raw={:?} status={advanced:?}",
        advanced.run,
        text(&advanced, "journal_lines")
    );

    // Replace the card with the same real journal machine but rules that do
    // not match any Rune Mysteries body. The machine must retain the raw read,
    // publish `unknown`, carry no invented signal ranges, and park.
    assert!(play.script_native_stop(&name, run), "stop synthetic run");
    wait_until("synthetic Quester stop", Duration::from_secs(20), || {
        handle.idle(&name)
    });
    let no_match_run = handle
        .start_test_script(
            &name,
            Box::new(Quester::new(
                api::selected::RunKey {
                    slot: 0,
                    run: 0,
                    session: 0,
                },
                no_match_path,
                Arc::clone(&quests),
            )),
            Some(selected),
        )
        .expect("install no-match Quester");
    play.wake(&name);
    let parked = wait_status(
        &play,
        &name,
        SYNTHETIC_TIMEOUT,
        "no-match journal parking",
        |status| {
            status.phase == NativePhase::Blocked
                && text(status, "rule") == "unknown"
                && text(status, "journal_lines") == text(&advanced, "journal_lines")
        },
    );
    assert_eq!(parked.run, no_match_run);
    assert!(matches!(
        field(&parked, "needs_read"),
        StatusValue::Truth(Truth::True)
    ));
    match field(&parked, "progress") {
        StatusValue::Quest(progress) => assert!(progress.signals.is_empty()),
        other => panic!("no-match progress is not Quest: {other:?}"),
    }
    println!(
        "synthetic no-match journal status: run={:?} raw={:?} status={parked:?}",
        parked.run,
        text(&parked, "journal_lines")
    );
    drop(play);
    drop(home);
}

#[test]
#[ignore = "requires LIVE=1 and the shared tunnelled local R289 engine"]
fn live_quester_cook_reaches_complete() {
    assert!(live(), "live_quester_cook_reaches_complete requires LIVE=1");
    let (home, play, setup, name) = launch_live(SetupMode::Cook, "cook");
    wait_relogged(&play, &setup);

    let handle = play.script_start_handle();
    play.script_start(&name, script::CompiledId("Quester"), serde_json::Map::new())
        .expect("start released Cook Quester card");
    wait_test_start(&handle, &name);
    wait_until("fresh Cook's Assistant completion", COOK_TIMEOUT, || {
        play.script_lifecycle_receipt(&name)
            .is_some_and(|receipt| receipt.state == script::ScriptTerminalState::Completed)
    });
    let receipt = play
        .script_lifecycle_receipt(&name)
        .expect("completed Cook receipt");
    println!("Cook terminal receipt: {receipt:?}");
    assert_eq!(receipt.state, script::ScriptTerminalState::Completed);
    assert_eq!(handle.run_state(&name), script::RunState::Idle);
    drop(play);
    drop(home);
}
