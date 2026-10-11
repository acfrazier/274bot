#![cfg(feature = "live-harness")]

//! Fixed eight-account clean-logout → fresh-login proof against Engine A.
//! Each account reaches scene 2, dwells while the login queue spacing expires,
//! logs out through its real `SlotArm`, and requests login immediately after
//! the off-world observation. A case passes only on a new scene-2 session
//! reached within its fixed deadline, with every post-logout handshake fresh.
//!
//! The ignored test records per-account latency, observations and code-5 retry
//! log receipts to `BOT_EVIDENCE_DIR`; failures remain in that receipt.
//! Example command, from this worktree with the repository's `isohome` wrapper:
//! `LIVE=1 BOT_CPU=1 BOT_NAV_BUILD=skip BOT_LIVE_NAME_PREFIX=rc5 ENGINE_DIR=/srv/dev/experiments/lostcity-289/engine NAV_PACK=/Volumes/dev-scratch/274bot-evidence/NAV-SHANTAY-EDGE/nav/289/bake-r2-a.navpack NAV_FLAGS=/Volumes/dev-scratch/274bot-evidence/NAV-SHANTAY-EDGE/nav/289/bake-r2-a.navflags BOT_EVIDENCE_DIR=/Volumes/dev-scratch/274bot-evidence/RELOG-CODE5-FIX cargo test --locked -p host-play --features live-harness,test-support --test relog_release_live relog_release_fixed_eight_account_cohort -- --ignored --nocapture --test-threads=1`
//! A separate ignored two-account proof checks that a primary's clean-exit
//! release wait does not stall a companion's concurrent login.
//! Example command, with the repository's `isohome` wrapper:
//! `LIVE=1 BOT_DEBUG=1 BOT_CPU=1 BOT_NAV_BUILD=skip BOT_LIVE_NAME_PREFIX=rc5c ENGINE_DIR=/srv/dev/experiments/lostcity-289/engine NAV_PACK=/Volumes/dev-scratch/274bot-evidence/NAV-SHANTAY-EDGE/nav/289/bake-r2-a.navpack NAV_FLAGS=/Volumes/dev-scratch/274bot-evidence/NAV-SHANTAY-EDGE/nav/289/bake-r2-a.navflags BOT_EVIDENCE_DIR=/Volumes/dev-scratch/274bot-evidence/RELOG-CODE5-FIX/concurrent-live isohome cargo test --locked -p host-play --features live-harness,test-support --test relog_release_live relog_release_concurrent_bot_is_not_stalled -- --ignored --nocapture --test-threads=1`

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use host_play::ProfileOptions;
use serde::Serialize;
use serde_json::json;
use vault::{Profile, ProfileSettings};

const COHORT_SIZE: usize = 8;
const INITIAL_READY_TIMEOUT: Duration = Duration::from_secs(180);
const OFFWORLD_TIMEOUT: Duration = Duration::from_secs(30);
const RELOG_READY_TIMEOUT: Duration = Duration::from_secs(120);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);
const INITIAL_DWELL: Duration = Duration::from_secs(2);
const COMPANION_RELEASE_STAGING: Duration = Duration::from_millis(1_300);
const COMPANION_LOGIN_OFFSET: Duration = Duration::from_millis(250);
const COMPANION_ATTEMPT_MAX_DELAY: Duration = Duration::from_millis(750);
const PRIMARY_LOGIN_IMMEDIATE_MAX_DELAY: Duration = Duration::from_millis(250);
const PRIMARY_RELEASE_MIN_DELAY: Duration = Duration::from_millis(1_100);
const ENGINE_A_DIR: &str = "/srv/dev/experiments/lostcity-289/engine";
const ENGINE_A_NAV_PACK: &str =
    "/Volumes/dev-scratch/274bot-evidence/NAV-SHANTAY-EDGE/nav/289/bake-r2-a.navpack";

#[derive(Debug, Clone)]
struct Observation {
    account: String,
    at: Instant,
    ingame: bool,
    scene: i32,
    session: u64,
}

#[derive(Debug, Clone)]
struct CapturedLog {
    at: Instant,
    slot: String,
    source: String,
    level: String,
    message: String,
}

struct Capture(Mutex<Vec<CapturedLog>>);

impl api::hostlog::Sink for Capture {
    fn record(&self, record: &api::hostlog::Record<'_>) {
        if [
            "relog decision",
            "session exit",
            "handshake ok",
            "login code",
        ]
        .iter()
        .any(|prefix| record.message.starts_with(prefix))
        {
            self.0.lock().unwrap().push(CapturedLog {
                at: Instant::now(),
                slot: record.slot.unwrap_or("-").to_owned(),
                source: format!("{:?}", record.source),
                level: format!("{:?}", record.level),
                message: record.message.to_owned(),
            });
        }
    }
}

static CAPTURE: Capture = Capture(Mutex::new(Vec::new()));

#[derive(Serialize)]
struct ObservationReceipt {
    elapsed_s: f64,
    ingame: bool,
    scene: i32,
    session: u64,
}

#[derive(Serialize)]
struct LogReceipt {
    elapsed_s: f64,
    slot: String,
    source: String,
    level: String,
    message: String,
}

#[derive(Serialize)]
struct AccountReceipt {
    account: String,
    passed: bool,
    failures: Vec<String>,
    observations: Vec<ObservationReceipt>,
    initial_ready_latency_s: Option<f64>,
    initial_session: Option<u64>,
    logout_requested_elapsed_s: Option<f64>,
    offworld_to_login_request_s: Option<f64>,
    login_requested_elapsed_s: Option<f64>,
    relog_ready_latency_s: Option<f64>,
    offworld_to_scene2_s: Option<f64>,
    relog_session: Option<u64>,
    fresh_relogin_attempts: Vec<LogReceipt>,
    code5_retry_logs: Vec<LogReceipt>,
    host_log: Vec<LogReceipt>,
}

impl AccountReceipt {
    fn new(account: &str) -> Self {
        Self {
            account: account.to_owned(),
            passed: false,
            failures: Vec::new(),
            observations: Vec::new(),
            initial_ready_latency_s: None,
            initial_session: None,
            logout_requested_elapsed_s: None,
            offworld_to_login_request_s: None,
            login_requested_elapsed_s: None,
            relog_ready_latency_s: None,
            offworld_to_scene2_s: None,
            relog_session: None,
            fresh_relogin_attempts: Vec::new(),
            code5_retry_logs: Vec::new(),
            host_log: Vec::new(),
        }
    }
}

fn log_receipt(log: &CapturedLog, origin: Instant) -> LogReceipt {
    LogReceipt {
        elapsed_s: log.at.duration_since(origin).as_secs_f64(),
        slot: log.slot.clone(),
        source: log.source.clone(),
        level: log.level.clone(),
        message: log.message.clone(),
    }
}

fn track_observation(
    event: &Observation,
    observations: &mut HashMap<String, Vec<Observation>>,
    latest: &mut HashMap<String, Observation>,
) {
    observations
        .entry(event.account.clone())
        .or_default()
        .push(event.clone());
    if latest
        .get(&event.account)
        .is_none_or(|previous| event.at >= previous.at)
    {
        latest.insert(event.account.clone(), event.clone());
    }
}

fn receive_observation(
    receiver: &mpsc::Receiver<Observation>,
    observations: &mut HashMap<String, Vec<Observation>>,
    latest: &mut HashMap<String, Observation>,
    deadline: Instant,
) -> Result<Observation, String> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err("deadline expired".into());
    }
    let event = receiver
        .recv_timeout(remaining)
        .map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => {
                "timed out waiting for a client observation".to_owned()
            }
            mpsc::RecvTimeoutError::Disconnected => "client observation channel closed".to_owned(),
        })?;
    track_observation(&event, observations, latest);
    Ok(event)
}

fn next_observation(
    receiver: &mpsc::Receiver<Observation>,
    queued: &mut VecDeque<Observation>,
    observations: &mut HashMap<String, Vec<Observation>>,
    latest: &mut HashMap<String, Observation>,
    deadline: Instant,
) -> Result<Observation, String> {
    if let Some(event) = queued.pop_front() {
        return Ok(event);
    }
    receive_observation(receiver, observations, latest, deadline)
}

fn until_observation(
    receiver: &mpsc::Receiver<Observation>,
    queued: &mut VecDeque<Observation>,
    observations: &mut HashMap<String, Vec<Observation>>,
    latest: &mut HashMap<String, Observation>,
    deadline: Instant,
    predicate: impl Fn(&Observation) -> bool,
) -> Result<Observation, String> {
    if let Some(index) = queued.iter().position(&predicate) {
        return queued
            .remove(index)
            .ok_or_else(|| "matched observation disappeared".to_owned());
    }
    loop {
        let event = receive_observation(receiver, observations, latest, deadline)?;
        if predicate(&event) {
            return Ok(event);
        }
        queued.push_back(event);
    }
}

fn observation_receipt(event: &Observation, origin: Instant) -> ObservationReceipt {
    ObservationReceipt {
        elapsed_s: event.at.duration_since(origin).as_secs_f64(),
        ingame: event.ingame,
        scene: event.scene,
        session: event.session,
    }
}

// Observation callbacks can wake the operator before tick_flags logs the exit.
// Arming login in that gap also changes the cosmetic operator-latch spelling;
// the transport reason must still be a server logout before the next attempt.
fn clean_exit_before_attempt(
    logs: &[&CapturedLog],
    logout_request: Instant,
    login_request: Instant,
) -> bool {
    let Some(attempt) = logs
        .iter()
        .find(|log| log.at >= login_request && log.message.starts_with("relog decision attempt"))
    else {
        return false;
    };
    logs.iter()
        .rev()
        .find(|log| {
            log.at >= logout_request
                && log.at <= attempt.at
                && log.message.starts_with("session exit reason=")
        })
        .is_some_and(|log| {
            log.message
                .starts_with("session exit reason=server_logout ")
                || log
                    .message
                    .starts_with("session exit reason=operator_logout transport=server_logout ")
        })
}

fn write_evidence(
    evidence_dir: &std::path::Path,
    engine_dir: &std::path::Path,
    nav_pack: &std::path::Path,
    nav_flags: &std::path::Path,
    names: &[String],
    receipts: &[AccountReceipt],
    started_at_unix_ms: u128,
) {
    let passed = receipts.iter().filter(|receipt| receipt.passed).count();
    let failed = receipts.len() - passed;
    let report = json!({
        "schema": "relog-release-live/v1",
        "status": if failed == 0 { "pass" } else { "fail" },
        "started_at_unix_ms": started_at_unix_ms,
        "fixed_cohort_size": COHORT_SIZE,
        "deadlines_s": {
            "initial_ready": INITIAL_READY_TIMEOUT.as_secs(),
            "offworld": OFFWORLD_TIMEOUT.as_secs(),
            "relog_ready": RELOG_READY_TIMEOUT.as_secs(),
            "cleanup": CLEANUP_TIMEOUT.as_secs(),
        },
        "mode": { "rendering": "CpuPix3D", "nav_build": "skip" },
        "cohort": names,
        "engine": {
            "name": "Engine A",
            "directory": engine_dir.display().to_string(),
            "game_endpoint": "127.0.0.1:44594",
            "asset_endpoint": "127.0.0.1:1080",
            "nav_pack": nav_pack.display().to_string(),
            "nav_flags": nav_flags.display().to_string(),
        },
        "counts": { "passed": passed, "failed": failed },
        "accounts": receipts,
    });
    let path = evidence_dir.join("relog-release-live.json");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&report).expect("serialize live evidence") + "\n",
    )
    .expect("write live evidence receipt");
    println!("RELOG-RELEASE-LIVE {}", path.display());
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("serialize live summary")
    );
}

fn engine_a_dir() -> PathBuf {
    std::env::var_os("WORLD_ENGINE_DIR")
        .or_else(|| std::env::var_os("ENGINE_DIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(ENGINE_A_DIR))
}

fn engine_a_nav_pack() -> PathBuf {
    std::env::var_os("NAV_PACK")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(ENGINE_A_NAV_PACK))
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, BOT_NAV_BUILD=skip and the shared Engine A"]
fn relog_release_fixed_eight_account_cohort() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"), "requires LIVE=1");
    assert_eq!(
        std::env::var("BOT_CPU").as_deref(),
        Ok("1"),
        "the live cohort must use CpuPix3D"
    );
    assert_eq!(
        std::env::var("BOT_NAV_BUILD").as_deref(),
        Ok("skip"),
        "the live cohort must use the reviewed Engine A nav pack"
    );
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX")
        .expect("set a unique BOT_LIVE_NAME_PREFIX for this live run");
    assert!(!prefix.is_empty(), "BOT_LIVE_NAME_PREFIX cannot be empty");

    let evidence_dir =
        PathBuf::from(std::env::var_os("BOT_EVIDENCE_DIR").expect("set BOT_EVIDENCE_DIR"));
    std::fs::create_dir_all(&evidence_dir).expect("create evidence directory");
    let started_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_millis();
    let origin = Instant::now();
    assert!(
        api::hostlog::install_sink(&CAPTURE),
        "run this live proof in its own test process"
    );

    let home = PathBuf::from(std::env::var_os("HOME").expect("isohome must provide HOME"));
    let engine_dir = engine_a_dir();
    let nav_pack = engine_a_nav_pack();
    let nav_flags = std::env::var_os("NAV_FLAGS")
        .map(PathBuf::from)
        .unwrap_or_else(|| nav_pack.with_extension("navflags"));
    let template = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(44594),
        http_port: Some(1080),
        engine_dir: Some(engine_dir.clone()),
        nav_pack: Some(nav_pack.clone()),
        nav_flags: Some(nav_flags.clone()),
        vault_path: Some(home.join("vault-relog-release-live")),
        ..ProfileOptions::default()
    }
    .resolve(None)
    .expect("resolve Engine A local-289 profile")
    .prepare_template()
    .expect("prepare Engine A profile template");

    let names = host_play::mint_live_names(COHORT_SIZE);
    assert_eq!(names.len(), COHORT_SIZE, "mint the fixed cohort");
    assert_eq!(
        names.iter().collect::<HashSet<_>>().len(),
        COHORT_SIZE,
        "the fixed live cohort must contain eight unique accounts"
    );
    assert!(
        names.iter().all(|name| name.starts_with(&prefix)),
        "all minted accounts must use BOT_LIVE_NAME_PREFIX"
    );
    let profiles = host_play::mint_live_entries(&names)
        .into_iter()
        .enumerate()
        .map(|(index, (username, password))| Profile {
            username,
            password: password.into(),
            uid: 289_205_000 + index as i32,
            settings: ProfileSettings::default(),
        })
        .collect();
    let mut receipts: Vec<_> = names.iter().map(|name| AccountReceipt::new(name)).collect();
    let (observation_tx, observation_rx) = mpsc::channel();
    let frame_observations = Arc::new(Mutex::new(HashMap::<String, (bool, i32, u64)>::new()));
    let callback_observations = Arc::clone(&frame_observations);
    let play = host_play::run_with_template(
        Arc::clone(&template),
        false,
        profiles,
        |_| (None, None),
        move |client, username, _| {
            let current = (client.ingame, client.scene_state, client.gens.session);
            let mut previous = callback_observations.lock().unwrap();
            if previous.get(username).copied() == Some(current) {
                return;
            }
            previous.insert(username.to_owned(), current);
            drop(previous);
            let _ = observation_tx.send(Observation {
                account: username.to_owned(),
                at: Instant::now(),
                ingame: current.0,
                scene: current.1,
                session: current.2,
            });
        },
    )
    .expect("start the fixed eight-slot Play cohort");
    let arms: Vec<_> = names
        .iter()
        .map(|name| {
            (
                name.clone(),
                play.arm(name).expect("each cohort account has a SlotArm"),
            )
        })
        .collect();

    let mut observations = HashMap::<String, Vec<Observation>>::new();
    let mut latest = HashMap::<String, Observation>::new();
    let mut queued = VecDeque::new();
    let mut initial_ready = HashMap::<String, Observation>::new();
    let initial_deadline = Instant::now() + INITIAL_READY_TIMEOUT;
    let mut initial_timeout = None;
    while initial_ready.len() < COHORT_SIZE {
        match until_observation(
            &observation_rx,
            &mut queued,
            &mut observations,
            &mut latest,
            initial_deadline,
            |event| event.ingame && event.scene == 2,
        ) {
            Ok(event) => {
                initial_ready.entry(event.account.clone()).or_insert(event);
            }
            Err(error) => {
                initial_timeout = Some(error);
                break;
            }
        }
    }

    if !initial_ready.is_empty() {
        // The final initial handshake must age past normal queue spacing so
        // the first clean relog probes release admission, not FIFO pacing.
        let dwell_deadline = Instant::now() + INITIAL_DWELL;
        while Instant::now() < dwell_deadline {
            if next_observation(
                &observation_rx,
                &mut queued,
                &mut observations,
                &mut latest,
                dwell_deadline,
            )
            .is_err()
            {
                break;
            }
        }
    }

    let mut logout_request_times = HashMap::<String, Instant>::new();
    let mut login_request_times = HashMap::<String, Instant>::new();
    for (index, name) in names.iter().enumerate() {
        let Some(first) = initial_ready.get(name).cloned() else {
            receipts[index].failures.push(format!(
                "initial scene-2 readiness not observed within {}s: {}",
                INITIAL_READY_TIMEOUT.as_secs(),
                initial_timeout.as_deref().unwrap_or("cohort incomplete")
            ));
            continue;
        };
        receipts[index].initial_ready_latency_s =
            Some(first.at.duration_since(origin).as_secs_f64());
        receipts[index].initial_session = Some(first.session);

        if !latest.get(name).is_some_and(|event| event.ingame) {
            receipts[index]
                .failures
                .push("account left game before the requested operator logout".into());
            continue;
        }

        let (account, arm) = &arms[index];
        let logout_requested_at = Instant::now();
        logout_request_times.insert(name.clone(), logout_requested_at);
        receipts[index].logout_requested_elapsed_s =
            Some(logout_requested_at.duration_since(origin).as_secs_f64());
        arm.request_logout();
        play.wake(account);

        let offline = match until_observation(
            &observation_rx,
            &mut queued,
            &mut observations,
            &mut latest,
            Instant::now() + OFFWORLD_TIMEOUT,
            |event| event.account == *name && !event.ingame && event.at >= logout_requested_at,
        ) {
            Ok(event) => event,
            Err(error) => {
                receipts[index]
                    .failures
                    .push(format!("off-world observation missing: {error}"));
                continue;
            }
        };

        // No deliberate dwell follows the off-world observation. This is the
        // operator's fresh login request, followed by the shared real FIFO.
        let login_requested_at = Instant::now();
        login_request_times.insert(name.clone(), login_requested_at);
        receipts[index].offworld_to_login_request_s =
            Some(login_requested_at.duration_since(offline.at).as_secs_f64());
        receipts[index].login_requested_elapsed_s =
            Some(login_requested_at.duration_since(origin).as_secs_f64());
        arm.arm_explicit_login();
        play.wake(account);

        match until_observation(
            &observation_rx,
            &mut queued,
            &mut observations,
            &mut latest,
            Instant::now() + RELOG_READY_TIMEOUT,
            |event| {
                event.account == *name
                    && event.at >= login_requested_at
                    && event.ingame
                    && event.scene == 2
                    && event.session != first.session
            },
        ) {
            Ok(ready) => {
                receipts[index].relog_ready_latency_s =
                    Some(ready.at.duration_since(login_requested_at).as_secs_f64());
                receipts[index].offworld_to_scene2_s =
                    Some(ready.at.duration_since(offline.at).as_secs_f64());
                receipts[index].relog_session = Some(ready.session);
            }
            Err(error) => receipts[index].failures.push(format!(
                "new session did not reach scene 2 within {}s: {error}",
                RELOG_READY_TIMEOUT.as_secs()
            )),
        }
    }

    // Leave every successfully re-entered account through the clean operator
    // path, then stop all slot threads before the test process exits.
    let mut cleanup_accounts = Vec::new();
    for (account, arm) in &arms {
        if latest.get(account).is_some_and(|event| event.ingame) {
            arm.request_logout();
            play.wake(account);
            cleanup_accounts.push(account.clone());
        }
    }
    let cleanup_deadline = Instant::now() + CLEANUP_TIMEOUT;
    while cleanup_accounts
        .iter()
        .any(|account| latest.get(account).is_some_and(|event| event.ingame))
    {
        if next_observation(
            &observation_rx,
            &mut queued,
            &mut observations,
            &mut latest,
            cleanup_deadline,
        )
        .is_err()
        {
            break;
        }
    }
    for (index, name) in names.iter().enumerate() {
        if latest.get(name).is_some_and(|event| event.ingame) {
            receipts[index]
                .failures
                .push("clean cohort shutdown did not reach off-world before its deadline".into());
        }
        if let Some((_, arm)) = arms.get(index) {
            arm.stop.store(true, Ordering::Relaxed);
            play.wake(name);
        }
    }
    drop(play);

    // Retain the observed state timeline and check logout/login receipts
    // against their original monotonic request instants.
    let all_logs = CAPTURE.0.lock().unwrap().clone();
    for (index, name) in names.iter().enumerate() {
        let account_logs: Vec<_> = all_logs.iter().filter(|log| log.slot == *name).collect();
        receipts[index].observations = observations
            .get(name)
            .map(|events| {
                events
                    .iter()
                    .map(|event| observation_receipt(event, origin))
                    .collect()
            })
            .unwrap_or_default();
        if receipts[index].offworld_to_login_request_s.is_some()
            && logout_request_times.get(name).is_some_and(|requested_at| {
                !login_request_times.get(name).is_some_and(|login_at| {
                    clean_exit_before_attempt(&account_logs, *requested_at, *login_at)
                })
            })
        {
            receipts[index]
                .failures
                .push("clean server-logout receipt before the relog handshake missing".into());
        }
        if let Some(requested_at) = login_request_times.get(name) {
            let attempts: Vec<_> = account_logs
                .iter()
                .filter(|log| {
                    log.at >= *requested_at && log.message.starts_with("relog decision attempt")
                })
                .collect();
            receipts[index].fresh_relogin_attempts = attempts
                .iter()
                .map(|log| log_receipt(log, origin))
                .collect();
            if attempts.is_empty() {
                receipts[index]
                    .failures
                    .push("no post-logout login-attempt receipt".into());
            } else if attempts
                .iter()
                .any(|log| !log.message.ends_with("reconnect=false"))
            {
                receipts[index]
                    .failures
                    .push("a post-logout handshake was not fresh (reconnect=false)".into());
            }
        }
        receipts[index].host_log = account_logs
            .iter()
            .map(|log| log_receipt(log, origin))
            .collect();
        receipts[index].code5_retry_logs = account_logs
            .iter()
            .filter(|log| {
                login_request_times
                    .get(name)
                    .is_some_and(|requested_at| log.at >= *requested_at)
                    && (log.message.starts_with("login code 5:")
                        || (log.message.contains("code=5")
                            && (log.message.starts_with("relog decision release_wait_ms=")
                                || log.message.starts_with("relog decision wait backoff_ms="))))
            })
            .map(|log| log_receipt(log, origin))
            .collect();
        receipts[index].passed = receipts[index].failures.is_empty()
            && receipts[index].initial_ready_latency_s.is_some()
            && receipts[index].offworld_to_login_request_s.is_some()
            && receipts[index].relog_ready_latency_s.is_some()
            && receipts[index].initial_session != receipts[index].relog_session;
    }

    write_evidence(
        &evidence_dir,
        &engine_dir,
        &nav_pack,
        &nav_flags,
        &names,
        &receipts,
        started_at_unix_ms,
    );
    assert!(
        receipts.iter().all(|receipt| receipt.passed),
        "one or more fixed-cohort accounts failed; see {}/relog-release-live.json",
        evidence_dir.display()
    );
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, BOT_NAV_BUILD=skip and the shared Engine A"]
fn relog_release_concurrent_bot_is_not_stalled() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"), "requires LIVE=1");
    assert_eq!(
        std::env::var("BOT_CPU").as_deref(),
        Ok("1"),
        "the live regression must use CpuPix3D"
    );
    assert_eq!(
        std::env::var("BOT_NAV_BUILD").as_deref(),
        Ok("skip"),
        "the live regression must use the reviewed Engine A nav pack"
    );
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX")
        .expect("set a unique BOT_LIVE_NAME_PREFIX for this live run");
    assert!(!prefix.is_empty(), "BOT_LIVE_NAME_PREFIX cannot be empty");

    let evidence_dir =
        PathBuf::from(std::env::var_os("BOT_EVIDENCE_DIR").expect("set BOT_EVIDENCE_DIR"));
    std::fs::create_dir_all(&evidence_dir).expect("create evidence directory");
    let started_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_millis();
    let origin = Instant::now();
    assert!(
        api::hostlog::install_sink(&CAPTURE),
        "run this live proof in its own test process"
    );

    let home = PathBuf::from(std::env::var_os("HOME").expect("isohome must provide HOME"));
    let engine_dir = engine_a_dir();
    let nav_pack = engine_a_nav_pack();
    let nav_flags = std::env::var_os("NAV_FLAGS")
        .map(PathBuf::from)
        .unwrap_or_else(|| nav_pack.with_extension("navflags"));
    let template = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(44594),
        http_port: Some(1080),
        engine_dir: Some(engine_dir.clone()),
        nav_pack: Some(nav_pack.clone()),
        nav_flags: Some(nav_flags.clone()),
        vault_path: Some(home.join("vault-relog-release-concurrent-live")),
        ..ProfileOptions::default()
    }
    .resolve(None)
    .expect("resolve Engine A local-289 profile")
    .prepare_template()
    .expect("prepare Engine A profile template");

    let names = host_play::mint_live_names(2);
    assert_eq!(names.len(), 2, "mint the primary and companion accounts");
    assert_eq!(
        names.iter().collect::<HashSet<_>>().len(),
        2,
        "the live accounts must be unique"
    );
    assert!(
        names.iter().all(|name| name.starts_with(&prefix)),
        "all minted accounts must use BOT_LIVE_NAME_PREFIX"
    );
    let primary = names[0].clone();
    let companion = names[1].clone();
    let profiles = host_play::mint_live_entries(&names)
        .into_iter()
        .enumerate()
        .map(|(index, (username, password))| Profile {
            username,
            password: password.into(),
            uid: 289_205_000 + index as i32,
            settings: ProfileSettings::default(),
        })
        .collect();
    let mut receipts: Vec<_> = names.iter().map(|name| AccountReceipt::new(name)).collect();
    let (observation_tx, observation_rx) = mpsc::channel();
    let frame_observations = Arc::new(Mutex::new(HashMap::<String, (bool, i32, u64)>::new()));
    let callback_observations = Arc::clone(&frame_observations);
    let play = host_play::run_with_template(
        Arc::clone(&template),
        false,
        profiles,
        |_| (None, None),
        move |client, username, _| {
            let current = (client.ingame, client.scene_state, client.gens.session);
            let mut previous = callback_observations.lock().unwrap();
            if previous.get(username).copied() == Some(current) {
                return;
            }
            previous.insert(username.to_owned(), current);
            drop(previous);
            let _ = observation_tx.send(Observation {
                account: username.to_owned(),
                at: Instant::now(),
                ingame: current.0,
                scene: current.1,
                session: current.2,
            });
        },
    )
    .expect("start the two-slot Play cohort");
    let arms: Vec<_> = names
        .iter()
        .map(|name| {
            (
                name.clone(),
                play.arm(name).expect("each account has a SlotArm"),
            )
        })
        .collect();

    let mut observations = HashMap::<String, Vec<Observation>>::new();
    let mut latest = HashMap::<String, Observation>::new();
    let mut queued = VecDeque::new();
    let mut initial_ready = HashMap::<String, Observation>::new();
    let initial_deadline = Instant::now() + INITIAL_READY_TIMEOUT;
    let mut initial_timeout = None;
    while initial_ready.len() < names.len() {
        match until_observation(
            &observation_rx,
            &mut queued,
            &mut observations,
            &mut latest,
            initial_deadline,
            |event| event.ingame && event.scene == 2 && !initial_ready.contains_key(&event.account),
        ) {
            Ok(event) => {
                initial_ready.insert(event.account.clone(), event);
            }
            Err(error) => {
                initial_timeout = Some(error);
                break;
            }
        }
    }
    for (index, name) in names.iter().enumerate() {
        if let Some(first) = initial_ready.get(name) {
            receipts[index].initial_ready_latency_s =
                Some(first.at.duration_since(origin).as_secs_f64());
            receipts[index].initial_session = Some(first.session);
        } else {
            receipts[index].failures.push(format!(
                "initial scene-2 readiness not observed within {}s: {}",
                INITIAL_READY_TIMEOUT.as_secs(),
                initial_timeout.as_deref().unwrap_or("cohort incomplete")
            ));
        }
    }

    if initial_ready.len() == names.len() {
        let dwell_deadline = Instant::now() + INITIAL_DWELL;
        while Instant::now() < dwell_deadline {
            if next_observation(
                &observation_rx,
                &mut queued,
                &mut observations,
                &mut latest,
                dwell_deadline,
            )
            .is_err()
            {
                break;
            }
        }
    }

    let mut logout_request_times = HashMap::<String, Instant>::new();
    let mut login_request_times = HashMap::<String, Instant>::new();
    let mut companion_offline = None;
    if initial_ready.len() == names.len()
        && latest.get(&companion).is_some_and(|event| event.ingame)
    {
        let logout_requested_at = Instant::now();
        logout_request_times.insert(companion.clone(), logout_requested_at);
        receipts[1].logout_requested_elapsed_s =
            Some(logout_requested_at.duration_since(origin).as_secs_f64());
        let (_, arm) = &arms[1];
        arm.request_logout();
        play.wake(&companion);
        match until_observation(
            &observation_rx,
            &mut queued,
            &mut observations,
            &mut latest,
            Instant::now() + OFFWORLD_TIMEOUT,
            |event| event.account == companion && !event.ingame && event.at >= logout_requested_at,
        ) {
            Ok(event) => companion_offline = Some(event),
            Err(error) => receipts[1]
                .failures
                .push(format!("companion off-world observation missing: {error}")),
        }
    } else if initial_ready.len() == names.len() {
        receipts[1]
            .failures
            .push("companion left game before its staging logout".into());
    }

    if let Some(offline) = companion_offline.as_ref() {
        let staging_deadline = offline.at + COMPANION_RELEASE_STAGING;
        while Instant::now() < staging_deadline {
            if next_observation(
                &observation_rx,
                &mut queued,
                &mut observations,
                &mut latest,
                staging_deadline,
            )
            .is_err()
            {
                break;
            }
        }
    }

    let mut primary_offline = None;
    let mut primary_login_requested_at = None;
    let mut companion_login_requested_at = None;
    if companion_offline.is_some() && latest.get(&primary).is_some_and(|event| event.ingame) {
        let logout_requested_at = Instant::now();
        logout_request_times.insert(primary.clone(), logout_requested_at);
        receipts[0].logout_requested_elapsed_s =
            Some(logout_requested_at.duration_since(origin).as_secs_f64());
        let (_, arm) = &arms[0];
        arm.request_logout();
        play.wake(&primary);
        match until_observation(
            &observation_rx,
            &mut queued,
            &mut observations,
            &mut latest,
            Instant::now() + OFFWORLD_TIMEOUT,
            |event| event.account == primary && !event.ingame && event.at >= logout_requested_at,
        ) {
            Ok(offline) => {
                let login_requested_at = Instant::now();
                let immediate_login_delay =
                    login_requested_at.saturating_duration_since(offline.at);
                receipts[0].offworld_to_login_request_s = Some(
                    login_requested_at
                        .saturating_duration_since(offline.at)
                        .as_secs_f64(),
                );
                receipts[0].login_requested_elapsed_s =
                    Some(login_requested_at.duration_since(origin).as_secs_f64());
                primary_login_requested_at = Some(login_requested_at);
                login_request_times.insert(primary.clone(), login_requested_at);
                primary_offline = Some(offline);
                arm.arm_explicit_login();
                play.wake(&primary);

                if immediate_login_delay > PRIMARY_LOGIN_IMMEDIATE_MAX_DELAY {
                    receipts[0].failures.push(format!(
                        "primary Log in request exceeded the immediate {} ms bound",
                        PRIMARY_LOGIN_IMMEDIATE_MAX_DELAY.as_millis()
                    ));
                }

                let companion_login_due_at = login_requested_at + COMPANION_LOGIN_OFFSET;
                while Instant::now() < companion_login_due_at {
                    if next_observation(
                        &observation_rx,
                        &mut queued,
                        &mut observations,
                        &mut latest,
                        companion_login_due_at,
                    )
                    .is_err()
                    {
                        break;
                    }
                }
                let companion_requested_at = Instant::now();
                receipts[1].offworld_to_login_request_s =
                    companion_offline.as_ref().map(|offline| {
                        companion_requested_at
                            .saturating_duration_since(offline.at)
                            .as_secs_f64()
                    });
                receipts[1].login_requested_elapsed_s =
                    Some(companion_requested_at.duration_since(origin).as_secs_f64());
                companion_login_requested_at = Some(companion_requested_at);
                login_request_times.insert(companion.clone(), companion_requested_at);
                let (_, companion_arm) = &arms[1];
                companion_arm.arm_explicit_login();
                play.wake(&companion);
            }
            Err(error) => receipts[0]
                .failures
                .push(format!("primary off-world observation missing: {error}")),
        }
    } else if companion_offline.is_some() {
        receipts[0]
            .failures
            .push("primary left game before its immediate relog".into());
    }

    if let (Some(primary_requested_at), Some(companion_requested_at)) =
        (primary_login_requested_at, companion_login_requested_at)
    {
        if companion_requested_at.saturating_duration_since(primary_requested_at)
            >= PRIMARY_RELEASE_MIN_DELAY
        {
            receipts[1].failures.push(
                "companion login request did not arrive during the primary release wait".into(),
            );
        }
        if let Some(offline) = companion_offline.as_ref() {
            if companion_requested_at.saturating_duration_since(offline.at)
                < COMPANION_RELEASE_STAGING
            {
                receipts[1]
                    .failures
                    .push("companion release staging interval was not met".into());
            }
        }
    }

    let requested_accounts = login_request_times.len();
    let mut relog_ready = HashMap::<String, Observation>::new();
    let relog_deadline = Instant::now() + RELOG_READY_TIMEOUT;
    while relog_ready.len() < requested_accounts {
        match until_observation(
            &observation_rx,
            &mut queued,
            &mut observations,
            &mut latest,
            relog_deadline,
            |event| {
                login_request_times
                    .get(&event.account)
                    .is_some_and(|requested_at| {
                        event.at >= *requested_at
                            && event.ingame
                            && event.scene == 2
                            && initial_ready
                                .get(&event.account)
                                .is_some_and(|first| event.session != first.session)
                    })
            },
        ) {
            Ok(event) => {
                relog_ready.insert(event.account.clone(), event);
            }
            Err(error) => {
                for name in names
                    .iter()
                    .filter(|name| login_request_times.contains_key(*name))
                    .filter(|name| !relog_ready.contains_key(*name))
                {
                    let index = names
                        .iter()
                        .position(|account| account == name)
                        .expect("requested account belongs to this cohort");
                    receipts[index].failures.push(format!(
                        "new session did not reach scene 2 within {}s: {error}",
                        RELOG_READY_TIMEOUT.as_secs()
                    ));
                }
                break;
            }
        }
    }
    for (index, name) in names.iter().enumerate() {
        if let Some(ready) = relog_ready.get(name) {
            let requested_at = login_request_times
                .get(name)
                .expect("ready account had a login request");
            receipts[index].relog_ready_latency_s = Some(
                ready
                    .at
                    .saturating_duration_since(*requested_at)
                    .as_secs_f64(),
            );
            receipts[index].offworld_to_scene2_s = if index == 0 {
                primary_offline
                    .as_ref()
                    .map(|offline| ready.at.saturating_duration_since(offline.at).as_secs_f64())
            } else {
                companion_offline
                    .as_ref()
                    .map(|offline| ready.at.saturating_duration_since(offline.at).as_secs_f64())
            };
            receipts[index].relog_session = Some(ready.session);
        }
    }

    let mut cleanup_accounts = Vec::new();
    for (account, arm) in &arms {
        if latest.get(account).is_some_and(|event| event.ingame) {
            arm.request_logout();
            play.wake(account);
            cleanup_accounts.push(account.clone());
        }
    }
    let cleanup_deadline = Instant::now() + CLEANUP_TIMEOUT;
    while cleanup_accounts
        .iter()
        .any(|account| latest.get(account).is_some_and(|event| event.ingame))
    {
        if next_observation(
            &observation_rx,
            &mut queued,
            &mut observations,
            &mut latest,
            cleanup_deadline,
        )
        .is_err()
        {
            break;
        }
    }
    for (index, name) in names.iter().enumerate() {
        if latest.get(name).is_some_and(|event| event.ingame) {
            receipts[index]
                .failures
                .push("clean two-slot shutdown did not reach off-world before its deadline".into());
        }
        if let Some((_, arm)) = arms.get(index) {
            arm.stop.store(true, Ordering::Relaxed);
            play.wake(name);
        }
    }
    drop(play);

    let all_logs = CAPTURE.0.lock().unwrap().clone();
    let mut first_attempt_times = HashMap::<String, Instant>::new();
    for (index, name) in names.iter().enumerate() {
        let account_logs: Vec<_> = all_logs.iter().filter(|log| log.slot == *name).collect();
        receipts[index].observations = observations
            .get(name)
            .map(|events| {
                events
                    .iter()
                    .map(|event| observation_receipt(event, origin))
                    .collect()
            })
            .unwrap_or_default();
        if let Some(requested_at) = login_request_times.get(name) {
            if !logout_request_times.get(name).is_some_and(|logout_at| {
                clean_exit_before_attempt(&account_logs, *logout_at, *requested_at)
            }) {
                receipts[index]
                    .failures
                    .push("clean server-logout receipt before the relog handshake missing".into());
            }
            let attempts: Vec<_> = account_logs
                .iter()
                .filter(|log| {
                    log.at >= *requested_at && log.message.starts_with("relog decision attempt")
                })
                .collect();
            receipts[index].fresh_relogin_attempts = attempts
                .iter()
                .map(|log| log_receipt(log, origin))
                .collect();
            if let Some(first_attempt) = attempts.iter().min_by_key(|log| log.at) {
                first_attempt_times.insert(name.clone(), first_attempt.at);
            }
            if attempts.is_empty() {
                receipts[index]
                    .failures
                    .push("no post-logout login-attempt receipt".into());
            } else if attempts
                .iter()
                .any(|log| !log.message.ends_with("reconnect=false"))
            {
                receipts[index]
                    .failures
                    .push("a post-logout handshake was not fresh (reconnect=false)".into());
            }
        }
        receipts[index].host_log = account_logs
            .iter()
            .map(|log| log_receipt(log, origin))
            .collect();
        receipts[index].code5_retry_logs = account_logs
            .iter()
            .filter(|log| {
                login_request_times
                    .get(name)
                    .is_some_and(|requested_at| log.at >= *requested_at)
                    && (log.message.starts_with("login code 5:")
                        || (log.message.contains("code=5")
                            && (log.message.starts_with("relog decision release_wait_ms=")
                                || log.message.starts_with("relog decision wait backoff_ms="))))
            })
            .map(|log| log_receipt(log, origin))
            .collect();
    }

    let primary_first_attempt = first_attempt_times.get(&primary).copied();
    let companion_first_attempt = first_attempt_times.get(&companion).copied();
    let companion_attempt_latency_s = companion_login_requested_at
        .zip(companion_first_attempt)
        .map(|(requested_at, attempt_at)| {
            attempt_at
                .saturating_duration_since(requested_at)
                .as_secs_f64()
        });
    if let Some(delay_s) = companion_attempt_latency_s {
        if delay_s > COMPANION_ATTEMPT_MAX_DELAY.as_secs_f64() {
            receipts[1].failures.push(format!(
                "companion's first fresh attempt took {delay_s:.3}s after its request (limit {:.3}s)",
                COMPANION_ATTEMPT_MAX_DELAY.as_secs_f64()
            ));
        }
    }
    let primary_clean_exit_before_companion_request = logout_request_times
        .get(&primary)
        .zip(companion_login_requested_at)
        .is_some_and(|(logout_at, companion_requested_at)| {
            all_logs.iter().any(|log| {
                log.slot == primary
                    && log.at >= *logout_at
                    && log.at <= companion_requested_at
                    && (log
                        .message
                        .starts_with("session exit reason=server_logout ")
                        || log.message.starts_with(
                            "session exit reason=operator_logout transport=server_logout ",
                        ))
            })
        });
    if !primary_clean_exit_before_companion_request {
        receipts[1].failures.push(
            "companion login request did not overlap the primary's observed clean-exit release wait"
                .into(),
        );
    }
    let companion_attempt_preceded_primary = companion_first_attempt
        .zip(primary_first_attempt)
        .is_some_and(|(companion_attempt, primary_attempt)| companion_attempt < primary_attempt);
    if !companion_attempt_preceded_primary {
        receipts[1]
            .failures
            .push("companion's first fresh attempt did not precede the primary's".into());
    }
    if let (Some(offline), Some(attempt_at)) = (primary_offline.as_ref(), primary_first_attempt) {
        let release_wait_s = attempt_at
            .saturating_duration_since(offline.at)
            .as_secs_f64();
        if release_wait_s < PRIMARY_RELEASE_MIN_DELAY.as_secs_f64() {
            receipts[0].failures.push(format!(
                "primary's fresh attempt arrived after only {release_wait_s:.3}s from off-world"
            ));
        }
    }
    for (index, name) in names.iter().enumerate() {
        receipts[index].passed = receipts[index].failures.is_empty()
            && receipts[index].initial_ready_latency_s.is_some()
            && receipts[index].offworld_to_login_request_s.is_some()
            && receipts[index].relog_ready_latency_s.is_some()
            && receipts[index].initial_session != receipts[index].relog_session;
    }

    let primary_login_delay_s = primary_login_requested_at
        .zip(primary_offline.as_ref().map(|offline| offline.at))
        .map(|(requested_at, offline_at)| {
            requested_at
                .saturating_duration_since(offline_at)
                .as_secs_f64()
        });
    let companion_login_offset_s = primary_login_requested_at
        .zip(companion_login_requested_at)
        .map(|(primary_at, companion_at)| {
            companion_at
                .saturating_duration_since(primary_at)
                .as_secs_f64()
        });
    let companion_release_age_at_login_s = companion_login_requested_at
        .zip(companion_offline.as_ref().map(|offline| offline.at))
        .map(|(requested_at, offline_at)| {
            requested_at
                .saturating_duration_since(offline_at)
                .as_secs_f64()
        });
    let primary_exit_to_attempt_s =
        primary_offline
            .as_ref()
            .zip(primary_first_attempt)
            .map(|(offline, attempt_at)| {
                attempt_at
                    .saturating_duration_since(offline.at)
                    .as_secs_f64()
            });
    let code5_retry_count: usize = receipts
        .iter()
        .map(|receipt| receipt.code5_retry_logs.len())
        .sum();
    let passed = receipts.iter().filter(|receipt| receipt.passed).count();
    let failed = receipts.len() - passed;
    let report = json!({
        "schema": "relog-release-concurrent-live/v1",
        "status": if failed == 0 { "pass" } else { "fail" },
        "started_at_unix_ms": started_at_unix_ms,
        "account_roles": {
            "primary": &primary,
            "companion": &companion,
        },
        "deadlines_s": {
            "initial_ready": INITIAL_READY_TIMEOUT.as_secs(),
            "offworld": OFFWORLD_TIMEOUT.as_secs(),
            "relog_ready": RELOG_READY_TIMEOUT.as_secs(),
            "cleanup": CLEANUP_TIMEOUT.as_secs(),
        },
        "timing_limits_s": {
            "companion_release_staging": COMPANION_RELEASE_STAGING.as_secs_f64(),
            "companion_login_offset_target": COMPANION_LOGIN_OFFSET.as_secs_f64(),
            "companion_attempt_max_delay": COMPANION_ATTEMPT_MAX_DELAY.as_secs_f64(),
            "primary_login_immediate_max_delay": PRIMARY_LOGIN_IMMEDIATE_MAX_DELAY.as_secs_f64(),
            "primary_release_min_delay": PRIMARY_RELEASE_MIN_DELAY.as_secs_f64(),
        },
        "concurrency": {
            "primary_login_delay_after_offworld_s": primary_login_delay_s,
            "companion_login_offset_after_primary_s": companion_login_offset_s,
            "companion_release_age_at_login_s": companion_release_age_at_login_s,
            "primary_exit_to_first_attempt_s": primary_exit_to_attempt_s,
            "companion_first_attempt_latency_s": companion_attempt_latency_s,
            "companion_first_attempt_preceded_primary": companion_attempt_preceded_primary,
            "primary_clean_exit_preceded_companion_request":
                primary_clean_exit_before_companion_request,
        },
        "mode": { "rendering": "CpuPix3D", "nav_build": "skip" },
        "engine": {
            "name": "Engine A",
            "directory": engine_dir.display().to_string(),
            "game_endpoint": "127.0.0.1:44594",
            "asset_endpoint": "127.0.0.1:1080",
            "nav_pack": nav_pack.display().to_string(),
            "nav_flags": nav_flags.display().to_string(),
        },
        "counts": {
            "passed": passed,
            "failed": failed,
            "code5_retry_observed": code5_retry_count != 0,
            "code5_retry_count": code5_retry_count,
        },
        "accounts": &receipts,
    });
    let path = evidence_dir.join("relog-release-concurrent-live.json");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&report).expect("serialize concurrent live evidence") + "\n",
    )
    .expect("write concurrent live evidence receipt");
    println!("RELOG-RELEASE-CONCURRENT-LIVE {}", path.display());
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("serialize concurrent live summary")
    );
    assert!(
        receipts.iter().all(|receipt| receipt.passed),
        "concurrent relog regression failed; see {}",
        path.display()
    );
}

#[test]
fn clean_exit_oracle_accepts_callback_log_race_and_rejects_dirty_or_unrelated_exits() {
    let origin = Instant::now();
    let logout = origin + Duration::from_secs(1);
    let login = origin + Duration::from_secs(2);
    let log = |at, message: &str| CapturedLog {
        at,
        slot: "alice".into(),
        source: "Host".into(),
        level: "Info".into(),
        message: message.into(),
    };
    let attempt = log(
        origin + Duration::from_secs(4),
        "relog decision attempt reconnect=false",
    );
    for message in [
        "session exit reason=operator_logout transport=server_logout local_idle_request_pending=false",
        "session exit reason=server_logout local_idle_request_pending=false",
    ] {
        for at in [logout, login - Duration::from_micros(1), login + Duration::from_micros(1)] {
            let clean = log(at, message);
            assert!(clean_exit_before_attempt(&[&clean, &attempt], logout, login));
        }
        for at in [logout - Duration::from_micros(1), attempt.at + Duration::from_micros(1)] {
            let unrelated = log(at, message);
            assert!(!clean_exit_before_attempt(&[&unrelated, &attempt], logout, login));
        }
        let clean = log(login, message);
        let dirty = log(login + Duration::from_micros(1), "session exit reason=transport_lost");
        assert!(!clean_exit_before_attempt(&[&clean, &dirty, &attempt], logout, login));
    }
    let dirty = log(
        login,
        "session exit reason=operator_logout transport=transport_lost",
    );
    assert!(!clean_exit_before_attempt(
        &[&dirty, &attempt],
        logout,
        login
    ));
    assert!(!clean_exit_before_attempt(&[&attempt], logout, login));
    assert!(!clean_exit_before_attempt(&[], logout, login));
}
