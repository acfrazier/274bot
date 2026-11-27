//! Ignored S8 qualification (SD1 §7, Main ruling 2026-11-27): the largest
//! admissible real-schema legacy vault, near the old 16 MiB reader ceiling,
//! converted through the real service on real SQLite and private files.
//! Writes `vault-budget.json` and `conversion.json` under `S8_OUT`. Run alone
//! with `--release --ignored --exact`.

use super::*;
use crate::local_state::{PIECE_BYTES, STAGE_BATCH_BYTES};
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

/// Every writer job's class and duration, and the WAL high-water seen after
/// each job.
#[derive(Clone, Default)]
struct Timeline {
    jobs: Arc<StdMutex<Vec<(&'static str, f64)>>>,
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
                jobs.lock()
                    .unwrap()
                    .push((class, elapsed.as_secs_f64() * 1000.0));
                let wal = wal_bytes(&root_path);
                let mut high = wal_high.lock().unwrap();
                *high = (*high).max(wal);
            }),
        )
    }

    fn take(&self) -> Vec<(&'static str, f64)> {
        std::mem::take(&mut *self.jobs.lock().unwrap())
    }

    fn wal_high(&self) -> u64 {
        *self.wal_high.lock().unwrap()
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

fn profile_jobs(jobs: &[(&'static str, f64)]) -> Vec<f64> {
    jobs.iter()
        .filter(|(class, _)| *class == "profiles")
        .map(|(_, ms)| *ms)
        .collect()
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

#[test]
#[ignore = "S8 qualification writes synthetic evidence; run alone with S8_OUT set, --release"]
fn s8_release_budget() {
    let _lease = crate::test_process::state();
    let out = PathBuf::from(std::env::var_os("S8_OUT").expect("S8_OUT"));
    std::fs::create_dir_all(&out).unwrap();
    let legacy = maximum_fixture();
    let expected = converted(legacy.clone());

    // 1. First conversion on a fresh root, timed per job.
    let main = Root::new("s8-budget-main");
    let target = main.target("local-274", "vault");
    write_legacy(target.path(), &legacy);
    let source_bytes = std::fs::metadata(target.path()).unwrap().len();
    assert!(source_bytes <= vault::MAX_VAULT_FILE_BYTES);
    let source_sha = file_sha(target.path());
    let started = Instant::now();
    let (_, opened) = vault::authenticate(&std::fs::read(target.path()).unwrap(), PASS).unwrap();
    let decrypt_ms = started.elapsed().as_secs_f64() * 1000.0;
    drop(opened);
    let timeline = Timeline::default();
    let mut root = main;
    root.service = None;
    root.service = Some(timeline.open(&root.root));
    root.state(&target);
    timeline.take();
    let pages_before = pages(&root.root);
    reset_sqlite_highwater();
    let started = Instant::now();
    let vault = root.unlock(&target, PASS).unwrap();
    let unlock_ms = started.elapsed().as_secs_f64() * 1000.0;
    let import_heap = sqlite_heap();
    let import_jobs = timeline.take();
    let import_sql = profile_jobs(&import_jobs);
    assert_eq!(
        joined(&vault),
        expected,
        "the conversion keeps every setting"
    );
    drop(vault);
    let pages_after = pages(&root.root);
    let occupied_after = pages_after.0 - pages_after.1;
    let wal_import_high = timeline.wal_high();
    let sealed_bytes = std::fs::metadata(target.path()).unwrap().len();

    // 2. Joined hydration after a restart.
    root.service = None;
    root.service = Some(timeline.open(&root.root));
    root.state(&target);
    timeline.take();
    reset_sqlite_highwater();
    let started = Instant::now();
    let joined_again = root.unlock(&target, PASS).unwrap();
    let hydration_ms = started.elapsed().as_secs_f64() * 1000.0;
    let hydration_heap = sqlite_heap();
    let hydration_jobs = profile_jobs(&timeline.take());
    assert_eq!(joined(&joined_again), expected, "restart readback");
    drop(joined_again);

    // 3. Reset cost and released storage, then a fresh vault.
    let started = Instant::now();
    reset(root.service(), &target).unwrap();
    let reset_ms = started.elapsed().as_secs_f64() * 1000.0;
    let reset_jobs = profile_jobs(&timeline.take());
    let created = create(root.service(), &target, "new-pass").unwrap();
    drop(created);
    let started = Instant::now();
    let empty = root.unlock(&target, "new-pass").unwrap();
    let collect_ms = started.elapsed().as_secs_f64() * 1000.0;
    let collect_jobs = profile_jobs(&timeline.take());
    assert_eq!(empty.profiles().count(), 0, "nothing retired returns");
    drop(empty);
    let pages_released = pages(&root.root);
    root.service = None;

    // 4. Crash recovery: after the SQL commit, and in the middle of staging.
    let mut recovery = Vec::new();
    for at in [
        "after-sql-commit",
        "stage-batch-committed",
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
        timeline.take();
        reset_sqlite_highwater();
        let started = Instant::now();
        let vault = crashed.unlock(&target, PASS).unwrap();
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        let heap = sqlite_heap();
        let jobs = profile_jobs(&timeline.take());
        assert_eq!(joined(&vault), expected, "{at}: no setting is lost");
        drop(vault);
        crashed.restart();
        assert!(crashed.state(&target).pending.is_empty(), "{at}");
        recovery.push(json!({
            "stopped_at": at,
            "staged_rows_left_invisible": staged,
            "next_unlock_ms": ms,
            "sql_jobs": stats(&jobs),
            "sqlite_heap_highwater": heap.1,
            "wal_bytes_high_water": timeline.wal_high(),
            "settings_lost": 0,
        }));
        for job in &jobs {
            assert!(*job <= JOB_THRESHOLD_MS, "{at}: a job took {job} ms");
        }
    }

    // 5. A second connection pins a reader through the whole conversion.
    let mut pinned = Root::new("s8-budget-pinned");
    let target = pinned.target("local-274", "vault");
    write_legacy(target.path(), &legacy);
    pinned.service = None;
    let timeline = Timeline::default();
    pinned.service = Some(timeline.open(&pinned.root));
    pinned.state(&target);
    let reader = inspect(&pinned.root);
    reader
        .execute_batch("BEGIN; SELECT count(*) FROM store_meta;")
        .unwrap();
    timeline.take();
    reset_sqlite_highwater();
    let started = Instant::now();
    let pinned_vault = pinned.unlock(&target, PASS).unwrap();
    let pinned_ms = started.elapsed().as_secs_f64() * 1000.0;
    let pinned_heap = sqlite_heap();
    let pinned_jobs = profile_jobs(&timeline.take());
    let pinned_wal = timeline.wal_high();
    assert_eq!(joined(&pinned_vault), expected);
    drop(pinned_vault);
    reader.execute_batch("COMMIT").unwrap();
    drop(reader);

    // 6. A database already near its page cap refuses the import before any
    // write; the legacy vault stays authoritative.
    let full = Root::new("s8-budget-full");
    let target = full.target("local-274", "vault");
    write_legacy(target.path(), &legacy);
    full.state(&target);
    {
        let conn = inspect(&full.root);
        conn.execute_batch("CREATE TABLE ballast (b BLOB)").unwrap();
        let ballast = vec![0u8; 1024 * 1024];
        for _ in 0..(sql_cap_pages() * 4096 / ballast.len() as u64 - 8) {
            conn.execute("INSERT INTO ballast (b) VALUES (?1)", [&ballast])
                .unwrap();
        }
    }
    let full_source = std::fs::read(target.path()).unwrap();
    let refused = match full.unlock(&target, PASS) {
        Err(JoinError::Refused { phase, detail }) => (phase, detail),
        other => panic!("a full database must refuse the import: {:?}", other.err()),
    };
    assert_eq!(refused.0, JoinPhase::Commit);
    assert!(refused.1.contains("Nothing was imported"), "{}", refused.1);
    assert_eq!(
        std::fs::read(target.path()).unwrap(),
        full_source,
        "source intact"
    );
    let full_state = full.state(&target);
    assert!(full_state.binding.is_none() && !full_state.unbound_rows);
    {
        let conn = inspect(&full.root);
        conn.execute_batch("DROP TABLE ballast").unwrap();
    }
    let after_refusal = full.unlock(&target, PASS).unwrap();
    assert_eq!(
        joined(&after_refusal),
        expected,
        "the import completes after room is made"
    );
    drop(after_refusal);

    let sql_jobs_max = import_sql
        .iter()
        .chain(&hydration_jobs)
        .chain(&reset_jobs)
        .chain(&collect_jobs)
        .chain(&pinned_jobs)
        .copied()
        .fold(0.0, f64::max);
    let heap_high = [import_heap.1, hydration_heap.1, pinned_heap.1]
        .into_iter()
        .max()
        .unwrap();
    let verdict = sql_jobs_max <= JOB_THRESHOLD_MS
        && heap_high <= crate::local_state::sqlite_heap_gate()
        && occupied_after <= sql_cap_pages()
        && wal_import_high <= crate::local_state::wal_max_bytes()
        && pinned_wal <= crate::local_state::wal_max_bytes();
    let budget = json!({
        "slice": "S8",
        "ruling": "Main 2026-11-27: PROFILE_PAGES = 4096 + 64 added to the database page cap; \
                   staged <= 1 MiB batches; one FULL activation transaction; 8 MiB SQLite \
                   C-heap gate and WAL caps unchanged",
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
            "whole_unlock_ms": unlock_ms,
            "sql_import": stats(&import_sql),
            "sealed_vault_bytes": sealed_bytes,
            "sqlite_heap_current": import_heap.0,
            "sqlite_heap_highwater": import_heap.1,
            "wal_bytes_high_water": wal_import_high,
        },
        "joined_hydration": {
            "whole_unlock_ms": hydration_ms,
            "sql_jobs": stats(&hydration_jobs),
            "sqlite_heap_highwater": hydration_heap.1,
        },
        "reset": {
            "reset_ms": reset_ms,
            "sql_jobs": stats(&reset_jobs),
            "collect_unlock_ms": collect_ms,
            "collect_jobs": stats(&collect_jobs),
            "pages_before_reset": pages_after.0,
            "free_pages_after_collect": pages_released.1,
            "released_pages": pages_released.1.saturating_sub(pages_after.1),
        },
        "crash_recovery": recovery,
        "pinned_reader": {
            "whole_unlock_ms": pinned_ms,
            "sql_jobs": stats(&pinned_jobs),
            "wal_bytes_high_water": pinned_wal,
            "sqlite_heap_highwater": pinned_heap.1,
        },
        "capacity_full": {
            "refused_phase": "profile row commit",
            "refusal": refused.1,
            "source_intact": true,
            "binding_written": false,
            "staged_rows_left": false,
            "imports_after_room_is_made": true,
            "os_enospc": "not exercised: no OS disk-full fault injection on this host; the \
                          admission refusal before any write is the measured path",
        },
        "database": {
            "page_size": 4096,
            "pages_before_import": pages_before.0,
            "page_count_after_import": pages_after.0,
            "occupied_pages_after_import": occupied_after,
            "page_cap_with_profiles": sql_cap_pages(),
            "s1_page_cap_comparator": crate::local_state::s1_page_cap(),
            "profile_pages": crate::local_state::profile_pages(),
        },
        "limits": {
            "whole_job_threshold_ms": JOB_THRESHOLD_MS,
            "sqlite_heap_gate_bytes": crate::local_state::sqlite_heap_gate(),
            "wal_max_bytes": crate::local_state::wal_max_bytes(),
        },
        "max_sql_job_ms": sql_jobs_max,
        "sqlite_heap_highwater_max": heap_high,
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
        let mut root = Root::new(&format!("s8-conversion-{label}"));
        let target = root.target("local-274", "vault");
        std::fs::write(target.path(), bytes).unwrap();
        let sha = file_sha(target.path());
        let legacy = legacy_profiles(bytes, pass);
        let expected = converted(legacy.clone());
        assert_eq!(joined(&root.unlock(&target, pass).unwrap()), expected);
        root.restart();
        let readback = joined(&root.unlock(&target, pass).unwrap()) == expected;
        sources.push(json!({
            "source": label,
            "sha256": sha,
            "bytes": bytes.len(),
            "shape": shape(&legacy),
            "conversion_moment": "first interactive or headless unlock (SD1 §5)",
            "committed_schema_version": user_version(&root.root),
            "binding_generation": root.state(&target).binding.map(|b| b.generation),
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
        "committed_schema_version": user_version(&root.root),
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
