//! Ignored S6a capacity qualification. It writes only synthetic private state.

use super::*;
use api::identity::Settlement;
use api::loadout::Loadout;
use api::local_state::{LoadoutEdit, LoadoutKey};
use crate::local_state::LoadoutSnapshot;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const LOADOUT_WAIT: Duration = Duration::from_secs(60);
const OPERATOR_STORE: &str = "operator-presets";
const CHECKPOINT_SAVE_CAP: usize = 600;

fn row(name: &str, item: String) -> Loadout {
    Loadout {
        name: name.to_owned(),
        worn: BTreeMap::new(),
        unassigned: vec![item],
        carry: Vec::new(),
    }
}

fn row_bytes(row: &Loadout) -> usize {
    files::row_bytes(row).unwrap()
}
fn migrated(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap().to_os_string();
    name.push(".migrated");
    path.with_file_name(name)
}

fn maximum_row(name: &str) -> Loadout {
    let limit = files::MAX_LOADOUT_ROW_BYTES;
    let mut low = 0;
    let mut high = limit;
    while low < high {
        let middle = low + (high - low + 1) / 2;
        if row_bytes(&row(name, "x".repeat(middle))) <= limit {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    let result = row(name, "x".repeat(low));
    assert_eq!(row_bytes(&result), limit);
    result
}

fn full_list(payload: usize, last: &Loadout) -> Vec<Loadout> {
    let mut rows = (0..files::MAX_LOADOUT_ROWS - 1)
        .map(|index| row(&format!("loadout-{index:03}"), format!("r{index:03}-{}", "x".repeat(payload)))
        )
        .collect::<Vec<_>>();
    rows.push(last.clone());
    rows
}

fn maximum_source_fixture() -> (usize, Vec<Loadout>, Vec<u8>, usize) {
    let last = maximum_row("loadout-511");
    let mut low = 0;
    let mut high = files::MAX_LOADOUTS_BYTES as usize / (files::MAX_LOADOUT_ROWS - 1);
    for _ in 0..20 {
        if low >= high { break; }
        let payload = low + (high - low + 1) / 2;
        if files::loadouts_bytes(&full_list(payload, &last)).unwrap().len() <= files::MAX_LOADOUTS_BYTES as usize {
            low = payload;
        } else {
            high = payload - 1;
        }
    }
    assert_eq!(low, high, "bounded binary search reaches the byte boundary");
    let rows = full_list(low, &last);
    let bytes = files::loadouts_bytes(&rows).unwrap();
    let rejected = files::loadouts_bytes(&full_list(low + 1, &last)).unwrap();
    assert_eq!(rows.len(), files::MAX_LOADOUT_ROWS);
    assert!(bytes.len() <= files::MAX_LOADOUTS_BYTES as usize);
    assert!(rejected.len() > files::MAX_LOADOUTS_BYTES as usize);
    assert_eq!(rows.iter().map(row_bytes).max(), Some(files::MAX_LOADOUT_ROW_BYTES));
    (low, rows, bytes, rejected.len())
}

fn row_with_round(name: &str, payload_bytes: usize, round: usize) -> Loadout {
    let prefix = format!("{round:08x}");
    assert!(payload_bytes >= prefix.len());
    row(name, format!("{prefix}{}", "z".repeat(payload_bytes - prefix.len())))
}

fn read_loadouts(service: &LocalStateService, key: &LoadoutKey) -> Result<LoadoutSnapshot, ErrorCode> {
    let deadline = Instant::now() + LOADOUT_WAIT;
    loop {
        let ticket = match service.start_loadout_readback(key) {
            Ok(ticket) => ticket,
            Err(error) if error.is_not_ready() && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            Err(error) => return Err(error),
        };
        match service.wait_loadout_readback(key, ticket, deadline.saturating_duration_since(Instant::now())) {
            Err(error) if error.is_not_ready() && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            result => return result,
        }
    }
}

fn save_loadout(service: &LocalStateService, edit: LoadoutEdit) -> Result<(), ErrorCode> {
    let request = match service.submit(DomainEdit::Loadout(edit)) {
        SubmitReceipt::Accepted { request } => request,
        SubmitReceipt::Refused(error) => return Err(error),
    };
    match settled(service, request, LOADOUT_WAIT) {
        Some(Settlement::Completed(_)) => Ok(()),
        Some(Settlement::Failed(error)) => Err(error),
        other => panic!("loadout write did not settle: {other:?}"),
    }
}

fn record_jobs(samples: &mut occupancy_tests::Samples, timeline: &Timeline, readback: Option<&str>) {
    for (class, milliseconds) in timeline.take() {
        let label = if class == "loadouts-readback" { readback.unwrap_or(class) } else { class };
        samples.entry(label.to_owned()).or_default().push(milliseconds);
    }
}

fn wal_frames_from_length(bytes: u64) -> u64 {
    if bytes <= 32 { 0 } else { (bytes - 32) / (sql::PAGE_SIZE + 24) }
}

fn page_count(root: &StateRoot) -> u64 {
    inspect(root)
        .query_row("PRAGMA page_count", [], |row| row.get::<_, i64>(0))
        .unwrap() as u64
}

fn import_counts(root: &StateRoot) -> (i64, i64) {
    inspect(root)
        .query_row(
            "SELECT (SELECT count(*) FROM loadout_head WHERE store='operator-presets'),\
                    (SELECT count(*) FROM import_ledger WHERE domain='loadouts-references' AND source='loadouts.json')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}
fn stored_body_stats(root: &StateRoot) -> (u64, u64, u64) {
    let (rows, bytes, largest): (i64, i64, i64) = inspect(root)
        .query_row(
            "SELECT count(*),coalesce(sum(length(body)),0),coalesce(max(length(body)),0) \
             FROM loadout_row WHERE store='operator-presets'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    (rows as u64, bytes as u64, largest as u64)
}

fn snapshot_payload_bytes(snapshot: &LoadoutSnapshot) -> usize {
    std::mem::size_of::<LoadoutSnapshot>() + std::mem::size_of_val(snapshot.rows())
        + snapshot.rows().iter().map(|row| {
            row.name.capacity()
                + row.unassigned.capacity() * std::mem::size_of::<String>()
                + row.unassigned.iter().map(String::capacity).sum::<usize>()
                + row.carry.capacity() * std::mem::size_of::<api::loadout::CarryEntry>()
                + row.carry.iter().map(|entry| entry.item.capacity()).sum::<usize>()
                + row.worn.iter().map(|(slot, item)| slot.capacity() + item.capacity()).sum::<usize>()
        }).sum::<usize>()
}

fn receipt(root: &StateRoot, version: u64, shape: &str, source_bytes: usize, restart: bool) -> Value {
    let (kind, evidence, path, digest): (String, String, String, Option<String>) = inspect(root)
        .query_row(
            "SELECT kind,evidence,path,NULLIF(lower(hex(digest)),'') FROM import_ledger \
             WHERE domain='loadouts-references' AND source='loadouts.json'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(evidence, "completed");
    json!({"domain":"loadouts-references","source":"loadouts.json","shape":shape,
        "moment":"first root-wide loadout hydration","committed_version":version,
        "kind":kind,"evidence":evidence,
        "evidence_filename":Path::new(&path).file_name().unwrap().to_string_lossy(),
        "source_digest_hex":digest,"source_bytes":source_bytes,"restart_readback":restart,
        "receipt":"local_state::sql_tests::loadouts_budget::loadouts_release_budget"})
}


#[test]
#[ignore = "S6a qualification writes synthetic evidence; run alone with S6_OUT set"]
fn loadouts_release_budget() {
    crate::test_process::isolate_state_test!(loadouts_release_budget);
    let _state_lease = crate::test_process::state();
    let out = PathBuf::from(std::env::var_os("S6_OUT").expect("S6_OUT"));
    std::fs::create_dir_all(&out).unwrap();
    let scratch = IsolatedEnv::enter("s6a-loadouts-budget");
    let root = root_of(&scratch);
    let source = root.loadouts_file();
    let (payload, expected, source_bytes, rejected_bytes) = maximum_source_fixture();
    write_private(&source, &source_bytes);
    reset_sqlite_highwater();
    let key = LoadoutKey::OperatorPresets;
    let mut samples = occupancy_tests::Samples::new();
    let (service, timeline) = Timeline::open_timed(&root);
    let started = Instant::now();
    let retained_before = read_loadouts(&service, &key).expect("maximum admitted fixture imports");
    assert!(!source.exists());
    assert_eq!(std::fs::read(migrated(&source)).unwrap(), source_bytes);
    occupancy_tests::record(&mut samples, "loadouts-import-residence", started.elapsed());
    record_jobs(&mut samples, &timeline, Some("loadouts-import"));
    assert_eq!(retained_before.rows(), expected.as_slice());
    assert_eq!(retained_before.rows().len(), files::MAX_LOADOUT_ROWS);
    assert_eq!(retained_before.unreadable_rows(), 0);
    assert_eq!(retained_before.lossy_rows(), 0);
    assert!(!retained_before.read_only());
    let cached = service.loadout_snapshot(&key).unwrap();
    assert_eq!(cached.rows().as_ptr(), retained_before.rows().as_ptr(), "cache-only reads share immutable rows");
    drop(cached);
    let heap_after_import = sqlite_heap();
    let pages_after_import = page_count(&root);
    let frames_after_import = wal_frames_from_length(wal_bytes(&root));
    let wal_after_import = wal_bytes(&root);
    let (stored_rows, stored_body_bytes, largest_body_bytes) = stored_body_stats(&root);
    assert_eq!(stored_rows, files::MAX_LOADOUT_ROWS as u64);
    let compact_source = serde_json::to_vec(&expected).unwrap().len();
    let canonical_compact = serde_json::to_vec(retained_before.rows()).unwrap().len();
    let max_row_bytes = expected.iter().map(row_bytes).max().unwrap();
    let import_version = retained_before.version().get();
    let mut receipts = Vec::<Value>::new();

    let row_payload_bytes = retained_before.rows()[files::MAX_LOADOUT_ROWS - 1].unassigned[0].len();
    let replacement = row_with_round("loadout-511", row_payload_bytes, 1);
    let mut expected_after = expected.clone();
    expected_after[files::MAX_LOADOUT_ROWS - 1] = replacement.clone();
    let after_bytes = files::loadouts_bytes(&expected_after).unwrap();
    assert_eq!(after_bytes.len(), source_bytes.len(), "same-size replacement fits a full byte-limit list");
    let old_alias = service.loadout_snapshot(&key).unwrap();
    assert_eq!(old_alias.rows().as_ptr(), retained_before.rows().as_ptr());
    let started = Instant::now();
    save_loadout(&service, LoadoutEdit::upsert(operator_identity(), Some(retained_before.version()), replacement.clone()).unwrap())
        .expect("maximum row replacement on a full list");
    occupancy_tests::record(&mut samples, "loadouts-save-residence", started.elapsed());
    record_jobs(&mut samples, &timeline, None);
    let retained_after = service.loadout_snapshot(&key).unwrap();
    assert_eq!(retained_after.rows(), expected_after.as_slice());
    assert_ne!(retained_before.rows().as_ptr(), retained_after.rows().as_ptr());
    assert_eq!(retained_before.rows()[files::MAX_LOADOUT_ROWS - 1], expected[files::MAX_LOADOUT_ROWS - 1]);
    assert_eq!(row_bytes(&replacement), files::MAX_LOADOUT_ROW_BYTES);
    let retained_old_bytes = snapshot_payload_bytes(&retained_before);
    let retained_new_bytes = snapshot_payload_bytes(&retained_after);
    let retained_unique_generations = 2;

    let oversized = row("loadout-511", "x".repeat(row_payload_bytes + 1));
    assert!(row_bytes(&oversized) > files::MAX_LOADOUT_ROW_BYTES);
    let over_row_error = match LoadoutEdit::upsert(
        operator_identity(),
        Some(retained_after.version()),
        oversized,
    ) {
        Ok(edit) => save_loadout(&service, edit).unwrap_err(),
        Err(error) => error,
    };
    let over_count_error = match LoadoutEdit::upsert(
        operator_identity(),
        Some(retained_after.version()),
        Loadout::new("loadout-512"),
    ) {
        Ok(edit) => save_loadout(&service, edit).unwrap_err(),
        Err(error) => error,
    };
    assert_eq!(service.loadout_snapshot(&key).unwrap().version(), retained_after.version());
    assert_eq!(service.loadout_snapshot(&key).unwrap().rows().len(), files::MAX_LOADOUT_ROWS);
    record_jobs(&mut samples, &timeline, None);

    let mut max_wal_bytes = wal_bytes(&root).max(wal_after_import);
    let mut max_wal_frames = wal_frames_from_length(max_wal_bytes).max(frames_after_import);
    let resets_before = service.truncate_counts_for_test().0;
    let mut checkpoint_replacements = 0;
    for round in 2..(CHECKPOINT_SAVE_CAP + 2) {
        let current = service.loadout_snapshot(&key).unwrap();
        let replacement = row_with_round("loadout-511", row_payload_bytes, round);
        let started = Instant::now();
        save_loadout(&service, LoadoutEdit::upsert(operator_identity(), Some(current.version()), replacement).unwrap())
            .expect("bounded checkpoint-driving replacement");
        occupancy_tests::record(&mut samples, "loadouts-save-residence", started.elapsed());
        record_jobs(&mut samples, &timeline, None);
        checkpoint_replacements = round - 1;
        let current_wal = wal_bytes(&root);
        max_wal_bytes = max_wal_bytes.max(current_wal);
        max_wal_frames = max_wal_frames.max(wal_frames_from_length(current_wal));
        if service.truncate_counts_for_test().0 > resets_before { break; }
    }
    record_jobs(&mut samples, &timeline, None);
    let checkpoint_resets = service.truncate_counts_for_test().0;
    assert!(checkpoint_resets > resets_before, "bounded saves did not trigger the idle WAL checkpoint");
    let checkpoint_deadline = Instant::now() + Duration::from_secs(2);
    while !samples.contains_key("wal-truncate") && Instant::now() < checkpoint_deadline {
        record_jobs(&mut samples, &timeline, None);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(samples.contains_key("wal-truncate"), "checkpoint timing was not observed");
    assert!(checkpoint_replacements <= CHECKPOINT_SAVE_CAP);
    let wal_after_checkpoint = wal_bytes(&root);
    max_wal_bytes = max_wal_bytes.max(wal_after_checkpoint);
    max_wal_frames = max_wal_frames.max(wal_frames_from_length(wal_after_checkpoint));
    let heap_after_checkpoint = sqlite_heap();
    let final_snapshot = service.loadout_snapshot(&key).unwrap();
    let final_version = final_snapshot.version().get();
    let pages_after_edits = page_count(&root);
    let pages_highwater = pages_after_import.max(pages_after_edits);
    let connections_first_lifetime = sql::connections_opened_for_test(&root);
    assert_eq!(connections_first_lifetime, 1);

    let started = Instant::now();
    drop(service);
    occupancy_tests::record(&mut samples, "service-close-shutdown", started.elapsed());
    drop(old_alias);
    drop(retained_before);
    drop(retained_after);
    let heap_after_close = sqlite_heap();

    let (reopened, read_timeline) = Timeline::open_timed(&root);
    let started = Instant::now();
    let warm = read_loadouts(&reopened, &key).unwrap();
    occupancy_tests::record(&mut samples, "loadouts-readback-residence", started.elapsed());
    record_jobs(&mut samples, &read_timeline, Some("loadouts-readback"));
    assert_eq!(warm.version().get(), final_version);
    assert_eq!(warm.rows(), final_snapshot.rows());
    receipts.push(receipt(
        &root,
        import_version,
        "512 current-shape rows; source and row byte bounds",
        source_bytes.len(),
        true,
    ));
    let started = Instant::now();
    drop(reopened);
    occupancy_tests::record(&mut samples, "service-close-shutdown", started.elapsed());
    assert_eq!(sql::connections_opened_for_test(&root), 2);
    drop(final_snapshot);

    let legacy_scratch = IsolatedEnv::enter("s6a-loadouts-expansion");
    let legacy_root = root_of(&legacy_scratch);
    let legacy_source = legacy_root.loadouts_file();
    let legacy = json!([{"name":"legacy-expanded","worn":[" mystery "],"carry":" supply "}]);
    let legacy_bytes = serde_json::to_vec_pretty(&legacy).unwrap();
    write_private(&legacy_source, &legacy_bytes);
    let (legacy_service, legacy_timeline) = Timeline::open_timed(&legacy_root);
    let legacy_snapshot = read_loadouts(&legacy_service, &key).unwrap();
    record_jobs(&mut samples, &legacy_timeline, Some("loadouts-import"));
    assert!(!legacy_source.exists());
    assert_eq!(std::fs::read(migrated(&legacy_source)).unwrap(), legacy_bytes);
    let legacy_compact_bytes = serde_json::to_vec(&legacy).unwrap().len();
    let legacy_canonical_bytes = serde_json::to_vec(legacy_snapshot.rows()).unwrap().len();
    let canonical_expansion = legacy_canonical_bytes as i64 - legacy_compact_bytes as i64;
    assert!(canonical_expansion > 0);
    assert!(!legacy_snapshot.read_only());
    receipts.push(receipt(&legacy_root, legacy_snapshot.version().get(), "supported worn-array and carry-string normalization", legacy_bytes.len(), false));
    drop(legacy_service);

    let over_scratch = IsolatedEnv::enter("s6a-loadouts-over-source");
    let over_root = root_of(&over_scratch);
    let over_source = over_root.loadouts_file();
    let over_bytes = files::loadouts_bytes(&full_list(payload + 1, &maximum_row("loadout-511"))).unwrap();
    write_private(&over_source, &over_bytes);
    let over_service = LocalStateService::open_process(over_root.clone());
    let source_error = read_loadouts(&over_service, &key).unwrap_err();
    assert!(!source_error.is_not_ready());
    assert_eq!(import_counts(&over_root), (0, 0));
    assert_eq!(std::fs::read(&over_source).unwrap(), over_bytes);
    drop(over_service);

    let count_scratch = IsolatedEnv::enter("s6a-loadouts-over-count");
    let count_root = root_of(&count_scratch);
    let count_source = count_root.loadouts_file();
    let too_many = (0..files::MAX_LOADOUT_ROWS + 1)
        .map(|index| Loadout::new(format!("row-{index}"))).collect::<Vec<_>>();
    let count_bytes = files::loadouts_bytes(&too_many).unwrap();
    write_private(&count_source, &count_bytes);
    let count_service = LocalStateService::open_process(count_root.clone());
    let count_error = read_loadouts(&count_service, &key).unwrap_err();
    assert!(!count_error.is_not_ready());
    assert_eq!(import_counts(&count_root), (0, 0));
    assert_eq!(std::fs::read(&count_source).unwrap(), count_bytes);
    drop(count_service);

    let recovery_scratch = IsolatedEnv::enter("s6a-loadouts-recovery");
    let recovery_root = root_of(&recovery_scratch);
    let recovery_source = recovery_root.loadouts_file();
    let recovery_bytes = br#"[{"name":"recover","worn":["cape"],"carry":"food"}]"#;
    write_private(&recovery_source, recovery_bytes);
    let status = std::process::Command::new(crate::test_process::executable())
        .args(["--ignored", "--exact", "local_state::sql_tests::loadouts_tests::loadout_import_evidence_crash_child"])
        .env("S6_LOADOUT_ROOT", recovery_root.path())
        .status().unwrap();
    assert_eq!(status.code(), Some(79));
    assert_eq!(sql::connections_opened_for_test(&recovery_root), 0);
    let (recovery_service, recovery_timeline) = Timeline::open_timed(&recovery_root);
    let started = Instant::now();
    let recovered = read_loadouts(&recovery_service, &key).unwrap();
    occupancy_tests::record(&mut samples, "loadouts-recovery-residence", started.elapsed());
    record_jobs(&mut samples, &recovery_timeline, Some("loadouts-recovery"));
    assert_eq!(recovered.rows()[0].name, "recover");
    assert!(!recovery_source.exists());
    assert_eq!(std::fs::read(migrated(&recovery_source)).unwrap(), recovery_bytes);
    assert_eq!(sql::connections_opened_for_test(&recovery_root), 1);
    receipts.push(receipt(&recovery_root, recovered.version().get(), "retry of pending import evidence after process exit", recovery_bytes.len(), true));
    drop(recovery_service);

    let heap = [heap_after_import, heap_after_checkpoint, heap_after_close, sqlite_heap()]
        .into_iter().max_by_key(|heap| heap.1).unwrap();
    let pages = pages_highwater;
    let wal = max_wal_bytes;
    let frames = max_wal_frames;
    let timing_summary = occupancy_tests::summarize(&samples);
    let owner_classes = ["loadouts-import", "loadouts-readback", "loadouts-save", "loadouts-recovery", "wal-truncate", "service-close-shutdown"];
    let whole_job_over_250ms = owner_classes.iter().flat_map(|class| samples.get(*class).into_iter().flatten())
        .filter(|&&milliseconds| milliseconds > 250.0).count();
    let budget = json!({
        "boundary":"existing SQLite caps; loadout staging reservation is DB_MAX_PAGES",
        "source_limit_bytes":files::MAX_LOADOUTS_BYTES,"row_limit":files::MAX_LOADOUT_ROWS,
        "row_bytes_limit":files::MAX_LOADOUT_ROW_BYTES,"source_bytes":source_bytes.len(),
        "maximum_source_payload_per_small_row":payload,"first_rejected_source_bytes":rejected_bytes,
        "first_rejected_source_refusal":source_error.to_string(),"first_rejected_source_intact":true,
        "first_rejected_source_heads_and_ledgers":import_counts(&over_root),
        "first_rejected_row_count":files::MAX_LOADOUT_ROWS+1,"row_count_refusal":count_error.to_string(),
        "row_count_source_intact":true,"row_count_heads_and_ledgers":import_counts(&count_root),
        "maximum_row_bytes":max_row_bytes,"maximum_legal_edit":{"operation":"replace one row at 64 KiB in a full 512-row list",
            "rows_before":files::MAX_LOADOUT_ROWS,"rows_after":expected_after.len(),
            "replacement_row_bytes":row_bytes(&replacement),"canonical_pretty_list_bytes":after_bytes.len(),
            "row_cap_refusal":over_row_error.to_string(),"row_count_refusal":over_count_error.to_string()},
        "canonical_expansion":{"maximum_current_list_source_compact_bytes":compact_source,
            "maximum_current_list_canonical_compact_bytes":canonical_compact,
            "maximum_current_list_expansion_bytes":canonical_compact as i64-compact_source as i64,
            "legacy_source_compact_bytes":legacy_compact_bytes,"legacy_canonical_compact_bytes":legacy_canonical_bytes,
            "legacy_expansion_bytes":canonical_expansion},
        "retained_snapshots":{"unique_row_generations":retained_unique_generations,
            "old_payload_capacity_bytes_estimate":retained_old_bytes,"new_payload_capacity_bytes_estimate":retained_new_bytes,
            "combined_payload_capacity_bytes_estimate":retained_old_bytes+retained_new_bytes,
            "estimate":"slice element and String/Vec capacities; excludes BTreeMap nodes and allocator/Arc metadata"},
        "sqlite":{"page_size":sql::PAGE_SIZE,"database_page_high_water":pages,"database_page_cap":sql::DB_MAX_PAGES,
            "loadout_transaction_page_reservation":sql::DB_MAX_PAGES,"wal_bytes_high_water":wal,"wal_bytes_cap":sql::WAL_MAX_BYTES,
            "wal_frames_high_water_observed":frames,"wal_frame_cap":sql::WAL_MAX_FRAMES,
            "frames_after_import":frames_after_import,"pages_after_import":pages_after_import,
            "stored_loadout_rows":stored_rows,"stored_row_bodies_bytes":stored_body_bytes,
            "largest_stored_row_body_bytes":largest_body_bytes,
            "checkpoint_resets":checkpoint_resets,"checkpoint_replacements":checkpoint_replacements,
            "wal_bytes_after_checkpoint":wal_after_checkpoint,
            "heap_current":heap.0,"heap_highwater":heap.1,"heap_cap":sql::SOFT_HEAP_LIMIT,
            "service_connections_first_lifetime":connections_first_lifetime,"service_connections_after_restart":2,
            "recovery_connections":1},
        "owner_timings_ms":timing_summary,"whole_owner_job_over_250ms":whole_job_over_250ms
    });
    std::fs::write(out.join("loadouts-budget.json"), serde_json::to_vec_pretty(&budget).unwrap()).unwrap();
    std::fs::write(out.join("loadouts-conversion.json"), serde_json::to_vec_pretty(&json!({"sources":receipts})).unwrap()).unwrap();

    assert!(pages <= sql::DB_MAX_PAGES);
    assert!(frames <= sql::WAL_MAX_FRAMES);
    assert!(wal <= sql::WAL_MAX_BYTES);
    assert!(heap.1 <= sql::SOFT_HEAP_LIMIT, "SQLite heap {} exceeds {}", heap.1, sql::SOFT_HEAP_LIMIT);
    assert_eq!(whole_job_over_250ms, 0, "loadout SQL owner job exceeded 250 ms");
    for class in owner_classes {
        assert!(samples.contains_key(class), "missing owner timing for {class}");
    }
}
