//! The occupancy gate's load model (plan §4.4): two OS processes on one
//! state root, each keeping eight queued preference saves and one active
//! save of D-sized documents, while the same process runs the bank and
//! Start jobs the gate protects (an exit save, a Start loadout readback and
//! a bank load). Every writer job is measured whole, from its selection to
//! its settled outcome, so the typed-view publication, the audit and the
//! terminal are inside the number.
//!
//! The module uses only the service's public surface and the writer's
//! job-timing hook, so the same file runs unchanged on the pinned JSON
//! build (the comparator) and on this one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex};

use api::local_state::{AccountKey, BankHint, ContentScope, DomainEdit, DomainIdentity};
use script::IsolatedEnv;
use serde_json::{json, Value};

use super::*;

const WAIT: Duration = Duration::from_secs(120);
/// Saves one process keeps in the writer's reach each round: one active
/// and eight queued.
const SAVES: usize = 9;
/// The writer's per-job pass threshold (plan §4.4): 250 ms.
pub(super) const THRESHOLD_MS: f64 = 250.0;
/// Rounds per process: 18 saves a round of about 25 frames each, so the log
/// passes the idle TRUNCATE trigger (W / 2 = 4,608 frames) more than once.
pub(super) const ROUNDS: usize = 24;

/// Per-class wall times, in milliseconds.
pub(super) type Samples = BTreeMap<String, Vec<f64>>;

fn ms(took: Duration) -> f64 {
    took.as_secs_f64() * 1000.0
}

pub(super) fn record(samples: &mut Samples, class: &str, took: Duration) {
    samples.entry(class.to_string()).or_default().push(ms(took));
}

/// `{samples, max_ms, p99_ms}` of one class.
pub(super) fn summary(values: &[f64]) -> Value {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let max = sorted.last().copied().unwrap_or_default();
    let p99 = sorted
        .get((sorted.len() * 99).div_ceil(100).saturating_sub(1))
        .copied()
        .unwrap_or_default();
    json!({ "samples": sorted.len(), "max_ms": max, "p99_ms": p99 })
}

pub(super) fn summarize(samples: &Samples) -> Value {
    Value::Object(
        samples
            .iter()
            .map(|(class, values)| (class.clone(), summary(values)))
            .collect(),
    )
}

pub(super) fn worst_ms(samples: &Samples) -> f64 {
    samples
        .values()
        .flatten()
        .copied()
        .fold(0.0, |worst, value| worst.max(value))
}

fn identity(domain: LocalStateDomain, account: &str) -> DomainIdentity {
    DomainIdentity::new(
        domain,
        AccountKey::try_new("local-289", account).unwrap(),
        ContentScope::try_new(client::io::ClientRevision::R289, "test-content").unwrap(),
    )
}

fn bank_hint(account: &str, at: u64) -> DomainEdit {
    DomainEdit::Bank(
        BankHint::try_new(
            identity(LocalStateDomain::BankHints, account),
            None,
            vec![(995, 10)],
            DurableTimestamp::from_unix_seconds(at),
        )
        .unwrap(),
    )
}

fn env_path(key: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(key).unwrap_or_else(|| panic!("{key} unset")))
}

fn env_number(key: &str) -> usize {
    std::env::var(key)
        .unwrap_or_else(|_| panic!("{key} unset"))
        .parse()
        .unwrap()
}

fn await_ready(preferences: &Preferences) {
    let deadline = Instant::now() + WAIT;
    loop {
        match preferences.snapshot() {
            Ok(_) => return,
            Err(error) if error.is_not_ready() => {}
            Err(error) => panic!("preferences refused: {error}"),
        }
        assert!(Instant::now() < deadline, "preferences never opened");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// One save of the load: 25 members of about 4 KiB, the size the writer's
/// queue holds nine of. Even saves write the same members in both processes
/// (the edits contend), odd saves write members of their own (disjoint).
/// Each save rewrites the whole D-sized document, whichever backend stores
/// it, because the root starts with a D-sized document ([`seed_document`]).
fn patch(round: usize, save: usize, index: usize) -> PreferencePatch {
    let mut patch = PreferencePatch::new();
    let (first, count) = if save.is_multiple_of(2) {
        (0, 25)
    } else {
        (50 + index * 100, 25)
    };
    for member in first..first + count {
        patch
            .set_member(
                &["bulk", &format!("m{member:03}")],
                Some(json!(format!(
                    "{round}-{index}-{save}-{}",
                    "w".repeat(3_900)
                ))),
            )
            .unwrap();
    }
    patch
}

/// Submit a bank hint (retrying while its cold store hydrates) and wait
/// for it: the queue residence a bank or exit save sees.
fn hint_residence(
    service: &LocalStateService,
    edit: impl Fn() -> DomainEdit,
) -> Result<Duration, String> {
    let started = Instant::now();
    let request = loop {
        match service.submit(edit()) {
            SubmitReceipt::Accepted { request } => break request,
            SubmitReceipt::Refused(ErrorCode::NotReady { .. }) => {
                if started.elapsed() > WAIT {
                    return Err("the bank store never hydrated".into());
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            SubmitReceipt::Refused(code) => return Err(format!("refused: {code}")),
        }
    };
    match service.wait_request(request, WAIT) {
        RequestState::Settled(Settlement::Completed(_)) => Ok(started.elapsed()),
        other => Err(format!("{other:?}")),
    }
}

/// One process of the load: `T53_ROUNDS` rounds, each with nine
/// preference saves in the writer's reach and the bank and Start jobs
/// beside them. It writes its report to `T53_OUT` (JSON).
#[test]
#[ignore = "child process of the occupancy load (two run at once)"]
fn occupancy_load_child() {
    let root = StateRoot::new(env_path("T53_ROOT"));
    let out = env_path("T53_OUT");
    let go = env_path("T53_GO");
    let index = env_number("T53_INDEX");
    let rounds = env_number("T53_ROUNDS");
    let service = LocalStateService::open_process(root);
    let preferences = service.preferences();
    await_ready(&preferences);
    let jobs = Arc::new(Mutex::new(Samples::new()));
    let sink = Arc::clone(&jobs);
    service.set_job_hook_for_test(Arc::new(move |class, took| {
        record(&mut sink.lock().unwrap(), class, took);
    }));
    let (entered_tx, entered) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let released = Mutex::new(released);
    let armed = Arc::new(AtomicBool::new(false));
    let hold = Arc::clone(&armed);
    service.set_write_stall_for_test(Arc::new(move || {
        if hold.swap(false, Ordering::SeqCst) {
            let _ = entered_tx.send(());
            let _ = released.lock().unwrap().recv_timeout(WAIT);
        }
    }));
    let deadline = Instant::now() + WAIT;
    while !go.exists() {
        assert!(Instant::now() < deadline, "the load never started");
        std::thread::sleep(Duration::from_millis(2));
    }
    let mut residence = Samples::new();
    let (mut accepted, mut rejected, mut saves_ok, mut occupied, mut ops_ok) =
        (0u64, 0u64, 0u64, 0u64, 0u64);
    let mut failures: Vec<String> = Vec::new();
    for round in 0..rounds {
        // One save held at its start, eight more accepted behind it: nine
        // in the writer's reach, as the gate's load states.
        armed.store(true, Ordering::SeqCst);
        let mut updates = Vec::new();
        for save in 0..SAVES {
            match preferences.begin_patch(patch(round, save, index)) {
                Ok(Update::Queued(update)) => {
                    accepted += 1;
                    updates.push(update);
                }
                Ok(Update::Durable) => {
                    accepted += 1;
                    saves_ok += 1;
                }
                Err(error) => {
                    rejected += 1;
                    failures.push(format!("save {save} of round {round} refused: {error}"));
                }
            }
            if save == 0 {
                entered
                    .recv_timeout(WAIT)
                    .expect("the first save is in flight");
            }
        }
        if service.preference_occupancy_for_test() == (SAVES - 1, true) {
            occupied += 1;
        }
        release.send(()).unwrap();
        // The jobs the gate protects, submitted together while the eight
        // wait: the exit save, the Start loadout readback and a bank load.
        // Each is measured from its own submission, so the numbers do not
        // depend on the order a backend happens to run them in.
        let at = 1_760_000_000 + round as u64;
        let exit = format!("bank{index}");
        let account = format!("bank{}", 10 + index * 10 + round);
        let (exit_save, start, bank_load) = std::thread::scope(|scope| {
            let exit_save = scope.spawn(|| hint_residence(&service, || bank_hint(&exit, at)));
            let start = scope.spawn(|| {
                let started = Instant::now();
                service
                    .readback_lease(
                        &identity(LocalStateDomain::LoadoutsReferences, "alice"),
                        WAIT,
                    )
                    .map(|_| started.elapsed())
            });
            let bank_load = scope.spawn(|| {
                let started = Instant::now();
                service
                    .readback_lease(&identity(LocalStateDomain::BankHints, &account), WAIT)
                    .map(|_| started.elapsed())
            });
            (
                exit_save.join().unwrap(),
                start.join().unwrap(),
                bank_load.join().unwrap(),
            )
        });
        match exit_save {
            Ok(took) => {
                ops_ok += 1;
                record(&mut residence, "bank-exit-save-residence", took);
            }
            Err(error) => failures.push(format!("exit save: {error}")),
        }
        match start {
            Ok(took) => {
                ops_ok += 1;
                record(&mut residence, "start-loadout-readback-residence", took);
            }
            Err(code) => failures.push(format!("start readback: {code}")),
        }
        match bank_load {
            Ok(took) => {
                ops_ok += 1;
                record(&mut residence, "bank-load-residence", took);
            }
            Err(code) => failures.push(format!("bank load: {code}")),
        }
        for mut update in updates {
            match update.wait(WAIT) {
                Some(Ok(())) => saves_ok += 1,
                Some(Err(error)) => failures.push(format!("a save failed: {error}")),
                None => failures.push("a save never settled".into()),
            }
        }
        // Follow the other process's commits: the one-second freshness
        // demand starts a refresh job (read, parse, typed view).
        std::thread::sleep(Duration::from_millis(1_100));
        let seen = preferences.snapshot().map(|snapshot| snapshot.version());
        let watch = Instant::now();
        while preferences.snapshot().map(|snapshot| snapshot.version()) == seen
            && watch.elapsed() < Duration::from_secs(3)
        {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    drop(preferences);
    let closing = Instant::now();
    drop(service);
    record(&mut residence, "service-close-shutdown", closing.elapsed());
    let mut classes = jobs.lock().unwrap().clone();
    classes.extend(residence);
    let report = json!({
        "index": index,
        "rounds": rounds,
        "accepted": accepted,
        "rejected": rejected,
        "saves_ok": saves_ok,
        "occupancy_rounds": occupied,
        "ops_ok": ops_ok,
        "failures": failures,
        "classes": classes,
    });
    std::fs::write(out, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
}

/// A running two-process load.
pub(super) struct Load {
    children: Vec<std::process::Child>,
    outs: Vec<PathBuf>,
    go: PathBuf,
    rounds: usize,
}

/// What the two processes did.
pub(super) struct LoadReport {
    pub(super) samples: Samples,
    pub(super) rounds: usize,
    pub(super) accepted: u64,
    pub(super) rejected: u64,
    pub(super) saves_ok: u64,
    pub(super) occupancy_rounds: u64,
    pub(super) ops_ok: u64,
    pub(super) failures: Vec<String>,
}

impl LoadReport {
    /// Every submitted save accepted and completed, nine in the writer's
    /// reach every round, every bank and Start job done, no failure.
    pub(super) fn assert_whole(&self) {
        let saves = (2 * self.rounds * SAVES) as u64;
        assert_eq!(self.rejected, 0, "{:?}", self.failures);
        assert_eq!(self.accepted, saves, "every save is accepted");
        assert_eq!(
            self.saves_ok, saves,
            "every save completes: {:?}",
            self.failures
        );
        assert_eq!(
            self.occupancy_rounds,
            (2 * self.rounds) as u64,
            "eight queued and one active in every round of both processes"
        );
        assert_eq!(
            self.ops_ok,
            (2 * self.rounds * 3) as u64,
            "{:?}",
            self.failures
        );
        assert!(self.failures.is_empty(), "{:?}", self.failures);
    }
}

/// A D-sized legacy document (250 members of about 4 KiB) at the root's
/// `panel-ui.json`, so both backends start from, and rewrite on every save,
/// the largest document they hold.
fn seed_document(root: &StateRoot) {
    let bulk: serde_json::Map<String, Value> = (0..250)
        .map(|member| {
            (
                format!("m{member:03}"),
                json!(format!("seed-{}", "d".repeat(3_900))),
            )
        })
        .collect();
    let bytes = serde_json::to_vec_pretty(&json!({ "bulk": bulk })).unwrap();
    vault::create_private_dir(root.path()).unwrap();
    vault::write_private_file(&root.preferences_file(), &bytes).unwrap();
}

/// Start the two processes on `root`; they wait for [`Load::go`].
pub(super) fn start_load(root: &StateRoot, dir: &Path, rounds: usize) -> Load {
    seed_document(root);
    let go = dir.join("occupancy-go");
    let mut children = Vec::new();
    let mut outs = Vec::new();
    for index in 0..2 {
        let out = dir.join(format!("occupancy-out-{index}.json"));
        children.push(
            crate::test_process::spawn(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--ignored",
                        "--exact",
                        "local_state::occupancy_tests::occupancy_load_child",
                    ])
                    .env("T53_ROOT", root.path())
                    .env("T53_OUT", &out)
                    .env("T53_GO", &go)
                    .env("T53_INDEX", index.to_string())
                    .env("T53_ROUNDS", rounds.to_string())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped()),
            )
            .unwrap(),
        );
        outs.push(out);
    }
    Load {
        children,
        outs,
        go,
        rounds,
    }
}

impl Load {
    pub(super) fn go(&self) {
        std::fs::write(&self.go, b"go").unwrap();
    }

    pub(super) fn finish(self) -> LoadReport {
        let mut report = LoadReport {
            samples: Samples::new(),
            rounds: self.rounds,
            accepted: 0,
            rejected: 0,
            saves_ok: 0,
            occupancy_rounds: 0,
            ops_ok: 0,
            failures: Vec::new(),
        };
        for (child, out) in self.children.into_iter().zip(self.outs) {
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "a load process failed: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let child: Value = serde_json::from_slice(&std::fs::read(out).unwrap()).unwrap();
            let count = |key: &str| child[key].as_u64().unwrap();
            report.accepted += count("accepted");
            report.rejected += count("rejected");
            report.saves_ok += count("saves_ok");
            report.occupancy_rounds += count("occupancy_rounds");
            report.ops_ok += count("ops_ok");
            for failure in child["failures"].as_array().unwrap() {
                report.failures.push(failure.as_str().unwrap().to_string());
            }
            for (class, values) in child["classes"].as_object().unwrap() {
                report.samples.entry(class.clone()).or_default().extend(
                    values
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_f64().unwrap()),
                );
            }
        }
        report
    }
}

/// The load alone, on this build's preferences backend: the measurement the
/// JSON comparator runs on the pinned build and the SQL gate extends.
/// Writes `T53_OCCUPANCY_OUT` (JSON) and fails when a class passes 250 ms.
#[test]
#[ignore = "the occupancy load on this build's backend; run alone with T53_OCCUPANCY_OUT"]
fn occupancy_load_report() {
    let out = env_path("T53_OCCUPANCY_OUT");
    let scratch = IsolatedEnv::enter("t53s4-occupancy-load");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let load = start_load(&root, &scratch.home, ROUNDS);
    load.go();
    let report = load.finish();
    let worst = worst_ms(&report.samples);
    let document = json!({
        "threshold_ms": THRESHOLD_MS,
        "worst_ms": worst,
        "os": std::env::consts::OS,
        "rounds": report.rounds,
        "accepted": report.accepted,
        "saves_ok": report.saves_ok,
        "occupancy_rounds": report.occupancy_rounds,
        "classes": summarize(&report.samples),
    });
    std::fs::write(&out, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
    report.assert_whole();
    assert!(worst <= THRESHOLD_MS, "a class passed 250 ms: {document}");
}
