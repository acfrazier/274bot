//! Ignored S8 qualification (SD1 §7, Main ruling 2026-11-27): the largest
//! admissible real-schema legacy vault, near the old 16 MiB reader ceiling,
//! converted through the real service on real SQLite and private files.
//! Writes `vault-budget.json` and `conversion.json` under `S8_OUT`. Run alone
//! with `--release --ignored --exact`.

use super::*;
use crate::local_state::{import_pages, Update, PIECE_BYTES, STAGE_BATCH_BYTES};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

/// The whole-job threshold every SQL job keeps (SD1 §7).
const JOB_THRESHOLD_MS: f64 = 250.0;
/// The plaintext the fixture targets: under the 16 MiB file ceiling after
/// the 29-byte header and the 16-byte tag.
const TARGET_PLAINTEXT: usize = 16_700_000;

fn sqlite_heap() -> (i64, i64) {
    // SAFETY: the bundled SQLite library is initialized by the open state
    // database. These calls only read process-wide counters.
    unsafe {
        (
            rusqlite::ffi::sqlite3_memory_used(),
            rusqlite::ffi::sqlite3_memory_highwater(0),
        )
    }
}

fn reset_sqlite_highwater() {
    // SAFETY: the bundled SQLite library is initialized. The flag only
    // resets its counter.
    unsafe {
        rusqlite::ffi::sqlite3_memory_highwater(1);
    }
}

fn inspect(root: &Path) -> Connection {
    let conn = Connection::open_with_flags(
        root.join("state").join("state.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    conn.busy_timeout(Duration::from_secs(5)).unwrap();
    conn
}

fn pages(root: &Path) -> (u64, u64) {
    let conn = inspect(root);
    let count: i64 = conn
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .unwrap();
    let free: i64 = conn
        .query_row("PRAGMA freelist_count", [], |row| row.get(0))
        .unwrap();
    (count as u64, free as u64)
}

fn wal_bytes(root: &Path) -> u64 {
    std::fs::metadata(root.join("state").join("state.sqlite3-wal")).map_or(0, |meta| meta.len())
}

/// One writer job: its class, milliseconds and SQLite C-heap high-water.
type Job = (&'static str, f64, i64);

/// Every writer job's class, duration and SQLite C-heap high-water (the
/// counter restarts after each job), and the WAL high-water seen after each
/// job.
#[derive(Clone, Default)]
struct Timeline {
    jobs: Arc<StdMutex<Vec<Job>>>,
    wal_high: Arc<StdMutex<u64>>,
}

impl Timeline {
    fn open(&self, root: &Path) -> Arc<LocalStateService> {
        let jobs = Arc::clone(&self.jobs);
        let wal_high = Arc::clone(&self.wal_high);
        let root_path = root.to_path_buf();
        LocalStateService::open_process_timed_for_test(
            StateRoot::new(root.to_path_buf()),
            Arc::new(move |class, elapsed| {
                // SAFETY: the bundled SQLite library is initialized by the
                // open state database; the call reads and restarts the
                // process-wide high-water counter.
                let heap = unsafe { rusqlite::ffi::sqlite3_memory_highwater(1) };
                jobs.lock()
                    .unwrap()
                    .push((class, elapsed.as_secs_f64() * 1000.0, heap));
                let wal = wal_bytes(&root_path);
                let mut high = wal_high.lock().unwrap();
                *high = (*high).max(wal);
            }),
        )
    }

    fn take(&self) -> Vec<Job> {
        std::mem::take(&mut *self.jobs.lock().unwrap())
    }

    /// The WAL high-water since the last call; restarts from zero.
    fn take_wal_high(&self) -> u64 {
        std::mem::take(&mut *self.wal_high.lock().unwrap())
    }
}

fn stats(values: &[f64]) -> Value {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let p99 = sorted
        .get(((sorted.len() as f64 * 0.99).ceil() as usize).saturating_sub(1))
        .copied()
        .unwrap_or(0.0);
    json!({
        "count": sorted.len(),
        "max_ms": sorted.last().copied().unwrap_or(0.0),
        "p99_ms": p99,
        "sum_ms": sorted.iter().sum::<f64>(),
    })
}

/// Every writer job class's duration statistics and its largest SQLite
/// C-heap high-water.
fn by_class(jobs: &[Job]) -> Value {
    let mut classes: BTreeMap<&str, (Vec<f64>, i64)> = BTreeMap::new();
    for (class, ms, heap) in jobs {
        let entry = classes.entry(class).or_default();
        entry.0.push(*ms);
        entry.1 = entry.1.max(*heap);
    }
    Value::Object(
        classes
            .into_iter()
            .map(|(class, (values, heap))| {
                let mut summary = stats(&values);
                summary["sqlite_heap_highwater"] = json!(heap);
                (class.to_string(), summary)
            })
            .collect(),
    )
}

fn occupied(root: &Path) -> u64 {
    let (count, free) = pages(root);
    count - free
}

/// The maxima every measured operation must keep.
#[derive(Default)]
struct Gate {
    max_job_ms: f64,
    heap_high: i64,
    wal_high: u64,
}

impl Gate {
    /// Run one operation and measure it: wall time, every writer job by
    /// class, the SQLite C-heap and physical WAL high-water, and the
    /// occupied pages before and after.
    fn measure<T>(
        &mut self,
        timeline: &Timeline,
        root: &Path,
        run: impl FnOnce() -> T,
    ) -> (T, Value) {
        timeline.take();
        timeline.take_wal_high();
        let before = occupied(root);
        reset_sqlite_highwater();
        let started = Instant::now();
        let value = run();
        let whole_ms = started.elapsed().as_secs_f64() * 1000.0;
        let jobs = timeline.take();
        // The writer restarts the counter after each job, so the operation's
        // high-water is the largest of its jobs' and what followed them.
        let heap = jobs
            .iter()
            .map(|(_, _, heap)| *heap)
            .fold(sqlite_heap().1, i64::max);
        let wal = timeline.take_wal_high().max(wal_bytes(root));
        let after = occupied(root);
        let max_job = jobs.iter().map(|(_, ms, _)| *ms).fold(0.0, f64::max);
        self.max_job_ms = self.max_job_ms.max(max_job);
        self.heap_high = self.heap_high.max(heap);
        self.wal_high = self.wal_high.max(wal);
        (
            value,
            json!({
                "whole_ms": whole_ms,
                "jobs_by_class": by_class(&jobs),
                "job_sequence_class_ms_heap": jobs,
                "max_job_ms": max_job,
                "sqlite_heap_highwater": heap,
                "wal_bytes_high_water": wal,
                "occupied_pages_before": before,
                "occupied_pages_after": after,
            }),
        )
    }
}

/// The largest admissible real-schema legacy vault: one profile with a
/// single settings bag larger than a staging batch, hundreds of profiles
/// with ordinary card bags (compiled Gatherer 4 and empty compiled entries
/// included), and thousands of default-sized profiles, filled to just under
/// the reader ceiling.
fn maximum_fixture() -> BTreeMap<String, Profile> {
    let bag = |value: Value| value.as_object().unwrap().clone();
    let mut profiles = BTreeMap::new();
    let mut add = |name: String, settings: ProfileSettings, index: usize| {
        profiles.insert(
            name.clone(),
            profile(
                &name,
                &format!("pw-{index:06}-s8"),
                274_100_000 + index as i32,
                settings,
            ),
        );
    };
    let mut big = ProfileSettings::default();
    big.script_settings.insert(
        "catalog:Huge".into(),
        bag(json!({"payload": "h".repeat(STAGE_BATCH_BYTES + 512 * 1024)})),
    );
    add("big0".into(), big, 0);
    for index in 1..=350 {
        let mut settings = ProfileSettings {
            world: Some(300 + (index % 40) as u16),
            lamp_skill: "magic".into(),
            ..ProfileSettings::default()
        };
        settings.script_settings.insert(
            format!("catalog:Card{index}"),
            bag(json!({"notes": "m".repeat(40_000), "count": index})),
        );
        settings.script_settings.insert(
            "compiled:Gatherer".into(),
            bag(json!({"schema_version": 4, "values": {"site": format!("s{index}")}})),
        );
        settings
            .script_settings
            .insert("compiled:Quester".into(), Map::new());
        add(format!("m{index:04}"), settings, index);
    }
    for index in 1..=2000 {
        add(
            format!("t{index:05}"),
            ProfileSettings::default(),
            1000 + index,
        );
    }
    let size = serde_json::to_vec(&profiles).unwrap().len();
    assert!(size < TARGET_PLAINTEXT, "{size}");
    // Fill to the target with one more profile; its bag length is exact.
    let probe = |filler: usize| {
        let mut settings = ProfileSettings::default();
        settings
            .script_settings
            .insert("catalog:Fill".into(), bag(json!({"f": "f".repeat(filler)})));
        settings
    };
    let base = {
        let mut with = profiles.clone();
        with.insert(
            "filler".into(),
            profile("filler", "pw-filler-s8", 274_199_999, probe(0)),
        );
        serde_json::to_vec(&with).unwrap().len()
    };
    profiles.insert(
        "filler".into(),
        profile(
            "filler",
            "pw-filler-s8",
            274_199_999,
            probe(TARGET_PLAINTEXT - base),
        ),
    );
    assert_eq!(
        serde_json::to_vec(&profiles).unwrap().len(),
        TARGET_PLAINTEXT
    );
    profiles
}

/// Write `profiles` as one legacy file with the old format's writer.
fn write_legacy(path: &Path, profiles: &BTreeMap<String, Profile>) {
    let vault = Vault::create(path, PASS).unwrap();
    let mut store = vault.store();
    let changes: Vec<VaultChange> = profiles
        .values()
        .cloned()
        .map(VaultChange::Upsert)
        .collect();
    store.commit_legacy(&changes).unwrap();
}

fn file_sha(path: &Path) -> String {
    let (_, digest) = vault::read_file(path).unwrap();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn shape(profiles: &BTreeMap<String, Profile>) -> Value {
    let bags: usize = profiles
        .values()
        .map(|profile| profile.settings.script_settings.len())
        .sum();
    let compiled: usize = profiles
        .values()
        .flat_map(|profile| profile.settings.script_settings.keys())
        .filter(|key| key.starts_with("compiled:"))
        .count();
    json!({"profiles": profiles.len(), "card_bags": bags, "compiled_entries": compiled})
}

fn user_version(root: &Path) -> i64 {
    inspect(root)
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap()
}

const WAIT: Duration = Duration::from_secs(120);

/// The pages the admission estimate gives `legacy`'s converted rows.
fn estimate_pages(legacy: &BTreeMap<String, Profile>) -> u64 {
    let mut items = Vec::new();
    for (index, profile) in converted(legacy.clone()).into_values().enumerate() {
        let account = account_of("local-274", &profile.username).unwrap();
        stage_items(
            index,
            NewProfileRow {
                account,
                username: profile.username,
                uid: profile.uid,
                settings: profile.settings,
            },
            &mut items,
        )
        .unwrap();
    }
    import_pages(&items)
}

/// A second process that holds a read transaction on `root`'s database
/// (so no checkpoint can reset the WAL) until it is stopped.
struct PinnedReader {
    child: std::process::Child,
    stop: PathBuf,
}

impl PinnedReader {
    fn start(root: &Path) -> Self {
        let ready = root.join("pinned-ready");
        let stop = root.join("pinned-stop");
        let child = std::process::Command::new(crate::test_process::executable())
            .args([
                "--ignored",
                "--exact",
                "profile_vault::tests::budget::s8_pinned_reader_child",
                "--nocapture",
            ])
            .env("S8_DB", root.join("state").join("state.sqlite3"))
            .env("S8_READY", &ready)
            .env("S8_STOP", &stop)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + WAIT;
        while !ready.exists() {
            assert!(
                Instant::now() < deadline,
                "the reader process pinned no snapshot"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        Self { child, stop }
    }

    /// Stop the reader; whether it held its snapshot to the end.
    fn stop(mut self) -> bool {
        std::fs::write(&self.stop, b"stop").unwrap();
        self.child.wait().unwrap().success()
    }
}

/// The second process of [`PinnedReader`].
#[test]
#[ignore = "child process of s8_release_budget"]
fn s8_pinned_reader_child() {
    let db = PathBuf::from(std::env::var_os("S8_DB").unwrap());
    let ready = PathBuf::from(std::env::var_os("S8_READY").unwrap());
    let stop = PathBuf::from(std::env::var_os("S8_STOP").unwrap());
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    conn.execute_batch("BEGIN").unwrap();
    let _: i64 = conn
        .query_row("SELECT count(*) FROM store_meta", [], |row| row.get(0))
        .unwrap();
    std::fs::write(ready, b"pinned").unwrap();
    let deadline = Instant::now() + Duration::from_secs(900);
    while !stop.exists() {
        assert!(Instant::now() < deadline, "never told to stop");
        std::thread::sleep(Duration::from_millis(20));
    }
    conn.execute_batch("COMMIT").unwrap();
}

/// Fill the domains the S1 cap covers near their admitted capacity: a
/// preference document near its 1 MiB bound (through the service) and all
/// 1,024 bank hints (`sql::MAX_BANK_HINTS`) at their widest, as
/// `a_full_bank_and_the_largest_preferences_fit_the_page_cap` fills them.
fn fill_prior_domains(root: &Path, service: &Arc<LocalStateService>) -> Value {
    let preferences = service.preferences();
    let deadline = Instant::now() + WAIT;
    while preferences.snapshot().is_err() {
        assert!(Instant::now() < deadline, "the preferences are ready");
        std::thread::sleep(Duration::from_millis(5));
    }
    let chunk = "x".repeat(900);
    let mut queued = Vec::new();
    for n in 0..1000 {
        if let Ok(Update::Queued(update)) =
            preferences.begin_set(&format!("pad-{n:04}"), json!(chunk))
        {
            queued.push(update);
        }
        if queued.len() == 8 || n == 999 {
            for mut update in queued.drain(..) {
                assert!(
                    matches!(update.wait(WAIT), Some(Ok(()))),
                    "a preference save is durable"
                );
            }
        }
    }
    let rows: Vec<u8> = (1..=api::bank_memory::MAX_HINT_ROWS as i32)
        .flat_map(|id| {
            let mut row = id.to_be_bytes().to_vec();
            row.extend_from_slice(&1_i32.to_be_bytes());
            row
        })
        .collect();
    let conn = inspect(root);
    conn.execute_batch("BEGIN").unwrap();
    for n in 0..1024u64 {
        conn.execute(
            "INSERT INTO bank_hint
                 (profile, account, revision, content, version, observed_at, rows, omitted)
             VALUES (?1, ?2, 289, ?3, ?4, 1, ?5, 0)",
            rusqlite::params![
                format!("filler-{}", n / 100),
                format!("f{}", n % 100),
                "s8-budget-content",
                (n + 1_000_000).to_be_bytes().to_vec(),
                rows
            ],
        )
        .unwrap();
    }
    conn.execute_batch("COMMIT").unwrap();
    json!({
        "preference_entries": 1000,
        "preference_entry_bytes": 900,
        "bank_hints": 1024,
        "bank_rows_each": api::bank_memory::MAX_HINT_ROWS,
        "occupied_pages": occupied(root),
        "s1_page_cap": crate::local_state::s1_page_cap(),
    })
}

/// The pages left for an import of `need` pages: the whole cap less the
/// occupied pages and `need`.
fn slack(root: &Path, need: u64) -> i64 {
    sql_cap_pages() as i64 - occupied(root) as i64 - need as i64
}

/// Grow a ballast table one page-sized row at a time until an import of
/// `need` pages no longer fits; with `admit`, give back rows until it just
/// fits again. Returns the slack left.
fn ballast_to_boundary(root: &Path, need: u64, admit: bool) -> i64 {
    let conn = inspect(root);
    conn.execute_batch("CREATE TABLE IF NOT EXISTS s8_ballast (b BLOB)")
        .unwrap();
    let row = vec![7u8; 3000];
    let bulk = slack(root, need) - 16;
    if bulk > 0 {
        conn.execute_batch("BEGIN").unwrap();
        for _ in 0..bulk {
            conn.execute("INSERT INTO s8_ballast (b) VALUES (?1)", [&row])
                .unwrap();
        }
        conn.execute_batch("COMMIT").unwrap();
    }
    while slack(root, need) >= 0 {
        conn.execute("INSERT INTO s8_ballast (b) VALUES (?1)", [&row])
            .unwrap();
    }
    while admit && slack(root, need) < 0 {
        conn.execute(
            "DELETE FROM s8_ballast WHERE rowid = (SELECT max(rowid) FROM s8_ballast)",
            [],
        )
        .unwrap();
    }
    slack(root, need)
}

#[test]
#[ignore = "S8 qualification writes synthetic evidence; run alone with S8_OUT set, --release"]
fn s8_release_budget() {
    let _lease = crate::test_process::state();
    let out = PathBuf::from(std::env::var_os("S8_OUT").expect("S8_OUT"));
    std::fs::create_dir_all(&out).unwrap();
    let legacy = maximum_fixture();
    let expected = converted(legacy.clone());
    let estimate = estimate_pages(&legacy);
    let mut gate = Gate::default();

    // 1. First conversion on a fresh root, with a second process pinning a
    // read snapshot through the whole conversion.
    let mut root = Root::new("s8-budget-main");
    let target = root.target("local-274", "vault");
    write_legacy(target.path(), &legacy);
    let source_bytes = std::fs::metadata(target.path()).unwrap().len();
    assert!(source_bytes <= vault::MAX_VAULT_FILE_BYTES);
    let source_sha = file_sha(target.path());
    let started = Instant::now();
    let (_, opened) = vault::authenticate(&std::fs::read(target.path()).unwrap(), PASS).unwrap();
    let decrypt_ms = started.elapsed().as_secs_f64() * 1000.0;
    drop(opened);
    let timeline = Timeline::default();
    root.service = None;
    root.service = Some(timeline.open(&root.root));
    root.state(&target);
    let reader = PinnedReader::start(&root.root);
    let (vault, conversion) = gate.measure(&timeline, &root.root, || {
        root.unlock(&target, PASS).unwrap()
    });
    assert!(reader.stop(), "the second process held its snapshot");
    assert_eq!(
        joined(&vault),
        expected,
        "the conversion keeps every setting"
    );
    drop(vault);
    let growth = conversion["occupied_pages_after"].as_u64().unwrap()
        - conversion["occupied_pages_before"].as_u64().unwrap();
    let sealed_bytes = std::fs::metadata(target.path()).unwrap().len();

    // 2. Joined hydration after a restart.
    root.service = None;
    root.service = Some(timeline.open(&root.root));
    root.state(&target);
    let (vault, hydration) = gate.measure(&timeline, &root.root, || {
        root.unlock(&target, PASS).unwrap()
    });
    assert_eq!(joined(&vault), expected, "restart readback");

    // 3. Post-seal writes at their largest: an ordinary edit, the largest
    // settings edit (the 1.5 MiB bag, staged as a copy) with a preference
    // save queued behind it, and the rename of that profile.
    let mut store = ProfileStore::joined(vault.store(), Arc::clone(root.service()));
    let mut card = vault.get("m0001").unwrap().clone();
    card.settings.world = Some(399);
    card.settings
        .script_settings
        .get_mut("catalog:Card1")
        .unwrap()
        .insert("notes".into(), json!("n".repeat(40_000)));
    let (_, normal_edit) = gate.measure(&timeline, &root.root, || {
        store.commit(&[VaultChange::Upsert(card.clone())]).unwrap()
    });
    let mut big = vault.get("big0").unwrap().clone();
    big.settings
        .script_settings
        .get_mut("catalog:Huge")
        .unwrap()
        .insert(
            "payload".into(),
            json!("g".repeat(STAGE_BATCH_BYTES + 512 * 1024)),
        );
    let preferences = root.service().preferences();
    let deadline = Instant::now() + WAIT;
    while preferences.snapshot().is_err() {
        assert!(Instant::now() < deadline, "the preferences are ready");
        std::thread::sleep(Duration::from_millis(5));
    }
    let (residence_ms, largest_edit) = gate.measure(&timeline, &root.root, || {
        std::thread::scope(|scope| {
            let edit = scope.spawn(|| store.commit(&[VaultChange::Upsert(big.clone())]));
            std::thread::sleep(Duration::from_millis(5));
            let started = Instant::now();
            if let Ok(Update::Queued(mut update)) = preferences.begin_set("s8-residence", json!(1))
            {
                assert!(matches!(update.wait(WAIT), Some(Ok(()))));
            }
            let residence = started.elapsed().as_secs_f64() * 1000.0;
            edit.join().unwrap().unwrap();
            residence
        })
    });
    // Repeated largest edits in one running session (no unlock between
    // them): each replaced 1.5 MiB copy is collected in the same session.
    let mut repeat = big.clone();
    let mut repeated = Vec::new();
    for round in 0..12 {
        let fill = if round % 2 == 0 { "r" } else { "s" };
        repeat
            .settings
            .script_settings
            .get_mut("catalog:Huge")
            .unwrap()
            .insert(
                "payload".into(),
                json!(fill.repeat(STAGE_BATCH_BYTES + 512 * 1024)),
            );
        let (_, measured) = gate.measure(&timeline, &root.root, || {
            store
                .commit(&[VaultChange::Upsert(repeat.clone())])
                .unwrap()
        });
        repeated.push(json!({
            "round": round + 1,
            "max_job_ms": measured["max_job_ms"],
            "sqlite_heap_highwater": measured["sqlite_heap_highwater"],
            "wal_bytes_high_water": measured["wal_bytes_high_water"],
            "occupied_pages_after": measured["occupied_pages_after"],
        }));
    }
    let repeated_pages: Vec<u64> = repeated
        .iter()
        .map(|round| round["occupied_pages_after"].as_u64().unwrap())
        .collect();
    let repeated_bounded = repeated_pages
        .iter()
        .all(|pages| *pages <= repeated_pages[0] + 16);
    let mut renamed = repeat.clone();
    renamed.username = "huge0".into();
    let (_, largest_rename) = gate.measure(&timeline, &root.root, || {
        store.commit_rename("big0", &renamed).unwrap()
    });
    drop(store);
    drop(vault);
    drop(preferences);
    let mut edited = expected.clone();
    edited.insert("m0001".into(), card);
    edited.remove("big0");
    edited.insert("huge0".into(), renamed);

    // 4. Collection of the retired copies at the next unlock.
    root.service = None;
    root.service = Some(timeline.open(&root.root));
    root.state(&target);
    let free_before_collect = pages(&root.root).1;
    let (vault, collection) = gate.measure(&timeline, &root.root, || {
        root.unlock(&target, PASS).unwrap()
    });
    assert_eq!(joined(&vault), edited, "every edit survives the restart");
    drop(vault);
    let released_by_collection = pages(&root.root).1.saturating_sub(free_before_collect);

    // 5. Reset cost and released storage, then a fresh vault.
    let pages_before_reset = pages(&root.root);
    let (_, reset_cost) = gate.measure(&timeline, &root.root, || {
        reset(root.service(), &target).unwrap()
    });
    drop(create(root.service(), &target, "new-pass").unwrap());
    let (empty, reset_collection) = gate.measure(&timeline, &root.root, || {
        root.unlock(&target, "new-pass").unwrap()
    });
    assert_eq!(empty.profiles().count(), 0, "nothing retired returns");
    drop(empty);
    let pages_after_reset = pages(&root.root);
    // SQLite's heap counter is process-wide: close each finished root's
    // service, so its page cache does not count against the next root.
    root.service = None;

    // 6. Crash recovery: in the middle of staging, after the SQL commit,
    // after the vault replacement.
    let mut recovery = Vec::new();
    for at in [
        "stage-batch-committed",
        "after-sql-commit",
        "after-vault-replace",
    ] {
        let mut crashed = Root::new(&format!("s8-budget-{at}"));
        let target = crashed.target("local-274", "vault");
        write_legacy(target.path(), &legacy);
        crashed.service = None;
        crash(&crashed, "unlock", at, &[]);
        let timeline = Timeline::default();
        crashed.service = Some(timeline.open(&crashed.root));
        let staged = crashed.state(&target).unbound_rows;
        // The cleanup the next unlock starts with, measured on its own.
        let (_, cleanup) = gate.measure(&timeline, &crashed.root, || {
            if staged {
                while crashed
                    .service()
                    .clear_unbound_profiles(target.server())
                    .unwrap()
                {}
            }
        });
        let (vault, resumed) = gate.measure(&timeline, &crashed.root, || {
            crashed.unlock(&target, PASS).unwrap()
        });
        assert_eq!(joined(&vault), expected, "{at}: no setting is lost");
        drop(vault);
        crashed.restart();
        let state = crashed.state(&target);
        assert!(state.pending.is_empty() && !state.unbound_rows, "{at}");
        recovery.push(json!({
            "stopped_at": at,
            "staged_rows_left_invisible": staged,
            "interrupted_staging_cleanup": cleanup,
            "next_unlock": resumed,
            "settings_lost": 0,
        }));
    }

    // 7. Coexistence: the preference and bank domains filled near their
    // capacity, then the largest conversion.
    let mut shared = Root::new("s8-budget-coexist");
    let target = shared.target("local-274", "vault");
    write_legacy(target.path(), &legacy);
    let timeline = Timeline::default();
    shared.service = None;
    shared.service = Some(timeline.open(&shared.root));
    let prior = fill_prior_domains(&shared.root, shared.service());
    shared.state(&target);
    let slack_before = slack(&shared.root, estimate);
    let (vault, coexisting) = gate.measure(&timeline, &shared.root, || {
        shared.unlock(&target, PASS).unwrap()
    });
    assert_eq!(
        joined(&vault),
        expected,
        "conversion beside the full domains"
    );
    drop(vault);
    let coexist_growth = coexisting["occupied_pages_after"].as_u64().unwrap()
        - coexisting["occupied_pages_before"].as_u64().unwrap();
    let combined = occupied(&shared.root);
    assert!(
        combined <= sql_cap_pages(),
        "the combined database stays within its cap"
    );
    shared.service = None;

    // 8. The admission boundary: the import fits with the smallest slack
    // the ballast leaves and converts; one page less refuses it before any
    // write, with the source intact.
    let mut edge = Vec::new();
    let mut admitted_root = None;
    for admit in [true, false] {
        let mut near = Root::new(&format!("s8-budget-edge-{admit}"));
        let target = near.target("local-274", "vault");
        write_legacy(target.path(), &legacy);
        let timeline = Timeline::default();
        near.service = None;
        near.service = Some(timeline.open(&near.root));
        fill_prior_domains(&near.root, near.service());
        near.state(&target);
        let left = ballast_to_boundary(&near.root, estimate, admit);
        let source = std::fs::read(target.path()).unwrap();
        let (result, measured) = gate.measure(&timeline, &near.root, || near.unlock(&target, PASS));
        if admit {
            assert!(left >= 0, "{left}");
            let vault = result.unwrap_or_else(|error| panic!("admitted at slack {left}: {error}"));
            assert_eq!(joined(&vault), expected);
            drop(vault);
            edge.push(json!({
                "slack_pages": left,
                "admitted": true,
                "converted": true,
                "measured": measured,
            }));
            near.service = None;
            admitted_root = Some((near, target));
        } else {
            assert!(left < 0, "{left}");
            let detail = match result {
                Err(JoinError::Refused {
                    phase: JoinPhase::Commit,
                    detail,
                }) => detail,
                other => panic!("one page short must refuse: {:?}", other.err()),
            };
            assert!(detail.contains("Nothing was imported"), "{detail}");
            assert_eq!(
                std::fs::read(target.path()).unwrap(),
                source,
                "source intact"
            );
            let state = near.state(&target);
            assert!(state.binding.is_none() && !state.unbound_rows);
            edge.push(json!({
                "slack_pages": left,
                "admitted": false,
                "refusal": detail,
                "source_intact": true,
                "rows_written": 0,
            }));
        }
    }

    // 9. Page-cap exhaustion (SQLITE_FULL from `max_page_count`): on the
    // root the boundary conversion left nearly full, a create larger than
    // the free pages fails inside its staging; nothing joins, the vault
    // image stays, and the next unlock collects the staged pages.
    let (mut near, target) = admitted_root.unwrap();
    near.service = Some(LocalStateService::open_process(StateRoot::new(
        near.root.clone(),
    )));
    let vault = near.unlock(&target, PASS).unwrap();
    let image = std::fs::read(target.path()).unwrap();
    let mut store = ProfileStore::joined(vault.store(), Arc::clone(near.service()));
    let mut huge = ProfileSettings::default();
    huge.script_settings.insert(
        "catalog:Fill".into(),
        json!({"f": "z".repeat(4 * 1024 * 1024)})
            .as_object()
            .unwrap()
            .clone(),
    );
    let full_error = store
        .commit(&[VaultChange::Upsert(Profile {
            username: "overflow".into(),
            password: "pw-overflow-s8".into(),
            uid: 274_199_998,
            settings: huge,
        })])
        .unwrap_err();
    assert!(full_error.contains("full"), "{full_error}");
    assert_eq!(
        std::fs::read(target.path()).unwrap(),
        image,
        "vault image intact"
    );
    drop(store);
    drop(vault);
    near.restart();
    let after_full = near.unlock(&target, PASS).unwrap();
    assert_eq!(
        joined(&after_full),
        expected,
        "nothing of the refused create joins"
    );
    drop(after_full);
    assert!(
        near.state(&target).prepared.is_empty(),
        "the staged pages were collected"
    );

    let verdict = gate.max_job_ms <= JOB_THRESHOLD_MS
        && gate.heap_high <= crate::local_state::sqlite_heap_gate()
        && gate.wal_high <= crate::local_state::wal_max_bytes()
        && estimate >= growth
        && estimate >= coexist_growth
        && growth <= crate::local_state::profile_pages()
        && repeated_bounded;
    let budget = json!({
        "slice": "S8",
        "ruling": "Main 2026-11-27: PROFILE_PAGES = 4,608 added to the S1 database page cap \
                   (4,160 + 4,608 = 8,768); staged <= 1 MiB transactions for the import, \
                   creates, large settings edits, renames and collection; one FULL \
                   activation transaction; 8 MiB SQLite C-heap gate and WAL caps unchanged",
        "build": "--locked --release",
        "fixture": {
            "kind": "largest admissible real-schema legacy vault (format 1, 0.2.9 writer shape)",
            "source_file_bytes": source_bytes,
            "reader_ceiling_bytes": vault::MAX_VAULT_FILE_BYTES,
            "plaintext_bytes": TARGET_PLAINTEXT,
            "largest_single_bag_bytes": STAGE_BATCH_BYTES + 512 * 1024,
            "shape": shape(&legacy),
            "piece_bytes": PIECE_BYTES,
            "stage_batch_bytes": STAGE_BATCH_BYTES,
        },
        "first_conversion": {
            "authenticate_decrypt_decode_ms": decrypt_ms,
            "pinned_reader": "a second process held one read transaction from before the \
                              conversion until after it",
            "measured": conversion,
            "sealed_vault_bytes": sealed_bytes,
        },
        "joined_hydration": hydration,
        "post_seal_writes": {
            "normal_edit": normal_edit,
            "largest_settings_edit": largest_edit,
            "preference_save_residence_during_largest_edit_ms": residence_ms,
            "largest_rename": largest_rename,
            "repeated_largest_edits_one_session": {
                "edits": repeated,
                "occupied_pages_first": repeated_pages[0],
                "occupied_pages_max": repeated_pages.iter().max(),
                "bounded": repeated_bounded,
            },
            "collection_at_next_unlock": collection,
            "pages_released_by_collection": released_by_collection,
        },
        "reset": {
            "reset": reset_cost,
            "collection_at_next_unlock": reset_collection,
            "pages_before_reset": pages_before_reset.0,
            "free_pages_after_collect": pages_after_reset.1,
            "released_pages": pages_after_reset.1.saturating_sub(pages_before_reset.1),
        },
        "crash_recovery": recovery,
        "coexistence": {
            "prior_domains": prior,
            "slack_pages_before_import": slack_before,
            "measured": coexisting,
            "occupied_page_growth": coexist_growth,
            "combined_occupied_pages": combined,
        },
        "admission_boundary": edge,
        "capacity_full": {
            "path": "SQLite max_page_count exhaustion (SQLITE_FULL) inside a post-seal create's \
                     staging on the nearly full boundary root",
            "error": full_error,
            "vault_image_intact": true,
            "joined_after_restart_unchanged": true,
            "staged_pages_collected": true,
            "os_enospc": "not exercised: no OS disk-full fault injection on this host",
        },
        "database": {
            "page_size": 4096,
            "estimated_import_pages": estimate,
            "occupied_page_growth_empty_root": growth,
            "occupied_page_growth_beside_full_domains": coexist_growth,
            "page_cap_with_profiles": sql_cap_pages(),
            "s1_page_cap_comparator": crate::local_state::s1_page_cap(),
            "profile_pages": crate::local_state::profile_pages(),
        },
        "limits": {
            "whole_job_threshold_ms": JOB_THRESHOLD_MS,
            "sqlite_heap_gate_bytes": crate::local_state::sqlite_heap_gate(),
            "wal_max_bytes": crate::local_state::wal_max_bytes(),
        },
        "max_sql_job_ms": gate.max_job_ms,
        "sqlite_heap_highwater_max": gate.heap_high,
        "wal_bytes_high_water_max": gate.wal_high,
        "verdict": if verdict { "pass" } else { "fail" },
    });
    std::fs::write(
        out.join("vault-budget.json"),
        serde_json::to_vec_pretty(&budget).unwrap(),
    )
    .unwrap();

    // conversion.json: the shipped fixtures and the maximum fixture.
    let mut sources = Vec::new();
    for (label, bytes, pass) in [
        ("legacy-0.1.9.vault", LEGACY_0_1_9, "bot"),
        ("vault-0.1.8.1.bin", LEGACY_0_1_8_1, "bot"),
        (
            "legacy-0.2.9-policies.vault",
            LEGACY_0_2_9,
            "v11c-legacy-fixture",
        ),
    ] {
        let mut fixture = Root::new(&format!("s8-conversion-{label}"));
        let target = fixture.target("local-274", "vault");
        std::fs::write(target.path(), bytes).unwrap();
        let sha = file_sha(target.path());
        let legacy = legacy_profiles(bytes, pass);
        let expected = converted(legacy.clone());
        assert_eq!(joined(&fixture.unlock(&target, pass).unwrap()), expected);
        fixture.restart();
        let readback = joined(&fixture.unlock(&target, pass).unwrap()) == expected;
        sources.push(json!({
            "source": label,
            "sha256": sha,
            "bytes": bytes.len(),
            "shape": shape(&legacy),
            "conversion_moment": "first interactive or headless unlock (SD1 §5)",
            "committed_schema_version": user_version(&fixture.root),
            "binding_generation": fixture.state(&target).binding.map(|b| b.generation),
            "evidence": "none: the source is replaced in place by the credential-only seal; \
                         old ciphertext is not retained after the first seal (SD1 §5)",
            "restart_readback_equal": readback,
            "receipt": "profile_vault::tests::shipped_fixtures_convert_and_both_stores_agree_after_restart",
        }));
    }
    sources.push(json!({
        "source": "synthetic maximum legacy vault (this run)",
        "sha256": source_sha,
        "bytes": source_bytes,
        "shape": shape(&legacy),
        "conversion_moment": "first unlock",
        "committed_schema_version": user_version(&shared.root),
        "evidence": "none: replaced in place by the seal; old ciphertext not retained",
        "restart_readback_equal": true,
        "receipt": "profile_vault::tests::budget::s8_release_budget",
    }));
    std::fs::write(
        out.join("conversion.json"),
        serde_json::to_vec_pretty(&json!({"slice": "S8", "sources": sources})).unwrap(),
    )
    .unwrap();
    assert!(
        verdict,
        "{}",
        serde_json::to_string_pretty(&budget).unwrap()
    );
}

fn sql_cap_pages() -> u64 {
    crate::local_state::database_page_cap()
}
