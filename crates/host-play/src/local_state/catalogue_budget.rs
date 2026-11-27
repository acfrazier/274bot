//! Synthetic release qualification. No script sources are opened or compiled.

use super::*;
use api::local_state::{
    CatalogueReferenceError, FileCardReference, MAX_CATALOGUE_FILE_BYTES, MAX_CATALOGUE_FILE_ROWS,
    MAX_CATALOGUE_ROOT_BYTES,
};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Barrier, Mutex};

type Timings = Arc<Mutex<occupancy_tests::Samples>>;

fn open_timed(root: &StateRoot, samples: &Timings) -> Arc<LocalStateService> {
    let samples = Arc::clone(samples);
    LocalStateService::open_process_timed_for_test(
        root.clone(),
        Arc::new(move |class, took| {
            occupancy_tests::record(&mut samples.lock().unwrap(), class, took);
        }),
    )
}

fn storage(root: &StateRoot) -> (u64, u64) {
    let conn = Connection::open(root.database_file()).unwrap();
    let pages: u32 = conn
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .unwrap();
    let wal = std::fs::metadata(sql::database_set(root)[1].clone()).map_or(0, |meta| meta.len());
    (u64::from(pages), wal)
}

fn checkpoint(service: &LocalStateService) -> (i64, i64, i64) {
    service
        .profile_sql("catalogue checkpoint", |db| {
            db.conn
                .as_ref()
                .unwrap()
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })
                .map_err(|error| sql::refused_by(error, "reference qualification checkpoint"))
        })
        .unwrap()
}

fn receipt(
    root: &StateRoot,
    source: &str,
    shape: &str,
    version: StoreVersion,
    bytes: usize,
) -> Value {
    let conn = Connection::open(root.database_file()).unwrap();
    let (kind, evidence): (String, String) = conn.query_row(
        "SELECT kind,evidence FROM import_ledger WHERE domain='catalogue-references' AND source=?1",
        [source], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(kind, "imported");
    assert_eq!(evidence, "completed");
    let evidence = format!("{source}.migrated");
    assert!(root.path().join(&evidence).is_file());
    json!({"source":source,"source_shape":shape,"source_bytes":bytes,
        "conversion_moment":"first hydration of this catalogue component, before restore or prompt",
        "committed_version":version.get(),"evidence_filename":evidence,
        "restart_readback":true,"test_receipt":"local_state::catalogue_budget::references_release_budget"})
}

#[test]
#[ignore = "S6c release qualification writes synthetic evidence; run alone with S6C_OUT set"]
fn references_release_budget() {
    crate::test_process::isolate_state_test!(references_release_budget);
    let _lease = crate::test_process::state();
    let out = PathBuf::from(std::env::var_os("S6C_OUT").expect("S6C_OUT"));
    std::fs::create_dir_all(&out).unwrap();
    let env = script::IsolatedEnv::enter("s6c-reference-budget");
    let root = StateRoot::new(env.dir.join("state-root"));
    std::fs::create_dir_all(root.path()).unwrap();
    let mut rows = vec![
        FileCardReference {
            name: String::new(),
            path: String::new()
        };
        MAX_CATALOGUE_FILE_ROWS
    ];
    rows[0].name = "a".into();
    rows[0].path = "b".into();
    rows[MAX_CATALOGUE_FILE_ROWS - 1].path = "c".into();
    let source = serde_json::to_vec(&rows).unwrap();
    assert_eq!(source.len(), MAX_CATALOGUE_FILE_BYTES);
    let saved_root = format!("/{}", "x".repeat(MAX_CATALOGUE_ROOT_BYTES - 1));
    for (name, bytes) in [
        ("js-scripts.json", source.as_slice()),
        ("rs2b0t-path", saved_root.as_bytes()),
        ("rs2b0t-import", b" deferred \n".as_slice()),
    ] {
        vault::write_private_file(&root.path().join(name), bytes).unwrap();
    }
    let loadouts = (0..128)
        .map(|index| api::loadout::Loadout {
            name: format!("loadout-{index:03}"),
            worn: Default::default(),
            unassigned: vec!["x".repeat(4096)],
            carry: Vec::new(),
        })
        .collect::<Vec<_>>();
    vault::write_private_file(
        &root.path().join("loadouts.json"),
        &serde_json::to_vec(&loadouts).unwrap(),
    )
    .unwrap();
    vault::write_private_file(
        &root.path().join("panel-ui.json"),
        &serde_json::to_vec(&json!({"qualification":"p".repeat(100 * 1024)})).unwrap(),
    )
    .unwrap();
    // SAFETY: this resets SQLite's process-wide diagnostic high-water counter.
    unsafe {
        rusqlite::ffi::sqlite3_memory_highwater(1);
    }
    let owner = Timings::default();
    let mut residence = occupancy_tests::Samples::new();
    let service = open_timed(&root, &owner);
    let key = api::local_state::LoadoutKey::OperatorPresets;
    let ticket = service.start_loadout_readback(&key).unwrap();
    let active_loadouts = service
        .wait_loadout_readback(&key, ticket, Duration::from_secs(60))
        .unwrap();
    assert_eq!(active_loadouts.rows().len(), loadouts.len());
    let port = service.catalogue_references();
    let started = Instant::now();
    let files = port.file_cards().unwrap();
    occupancy_tests::record(&mut residence, "file-store-hydration", started.elapsed());
    assert_eq!(files.value.as_ref(), rows.as_slice());
    let saved = port.saved_root().unwrap();
    assert_eq!(saved.value, Some(PathBuf::from(&saved_root)));
    let deferred = port.import_deferred().unwrap();
    assert!(deferred.value);
    let versions = [files.version, saved.version, deferred.version];
    let mut peak = storage(&root);
    let import_storage = peak;
    let receipts = vec![
        receipt(&root, "js-scripts.json", "ordered maximum-row list; same-stem, unresolved and unsafe references are retained without opening", files.version, source.len()),
        receipt(&root, "rs2b0t-path", "absolute lexical root at the 8192-byte limit; checkout need not exist", saved.version, saved_root.len()),
        receipt(&root, "rs2b0t-import", "trimmed deferred token", deferred.version, b" deferred \n".len()),
    ];
    // Three concurrent writers start from independent component versions.
    let barrier = Arc::new(Barrier::new(4));
    let mut changed = rows;
    changed[0].name = "z".into();
    let changed: Arc<[FileCardReference]> = changed.into();
    let mut handles = Vec::new();
    for component in 0..3 {
        let barrier = Arc::clone(&barrier);
        let port = Arc::clone(&port);
        let changed = Arc::clone(&changed);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            let started = Instant::now();
            let version = match component {
                0 => port.save_file_cards(versions[0], changed),
                1 => port.save_root(Some(versions[1]), Some("/chosen/checkout".into())),
                _ => port.save_deferred(Some(versions[2]), false),
            }
            .unwrap();
            (version, started.elapsed())
        }));
    }
    barrier.wait();
    let edits = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    for (component, (_, took)) in edits.iter().enumerate() {
        occupancy_tests::record(
            &mut residence,
            &format!("component-{component}-contended-edit"),
            *took,
        );
    }
    assert!(matches!(
        port.save_file_cards(files.version, Arc::clone(&files.value)),
        Err(CatalogueReferenceError::Conflict)
    ));
    assert!(matches!(
        port.save_root(Some(saved.version), None),
        Err(CatalogueReferenceError::Conflict)
    ));
    assert!(matches!(
        port.save_deferred(Some(deferred.version), true),
        Err(CatalogueReferenceError::Conflict)
    ));
    let now = storage(&root);
    peak = (peak.0.max(now.0), peak.1.max(now.1));
    drop(port);
    drop(service);
    let service = open_timed(&root, &owner);
    let port = service.catalogue_references();
    let started = Instant::now();
    let restarted = port.file_cards().unwrap();
    occupancy_tests::record(
        &mut residence,
        "file-store-restart-hydration",
        started.elapsed(),
    );
    assert_eq!(restarted.value, changed);
    assert_eq!(restarted.version, edits[0].0);
    assert_eq!(port.saved_root().unwrap().version, edits[1].0);
    assert_eq!(port.import_deferred().unwrap().version, edits[2].0);
    assert!(!port.import_deferred().unwrap().value);
    // A single large string maximizes durable bytes, unlike the maximum-row fixture.
    let single: Arc<[FileCardReference]> = vec![FileCardReference {
        name: String::new(),
        path: "p".repeat(MAX_CATALOGUE_FILE_BYTES - 23),
    }]
    .into();
    assert_eq!(
        serde_json::to_vec(single.as_ref()).unwrap().len(),
        MAX_CATALOGUE_FILE_BYTES
    );
    let started = Instant::now();
    let large_version = port
        .save_file_cards(restarted.version, Arc::clone(&single))
        .unwrap();
    occupancy_tests::record(&mut residence, "largest-body-edit", started.elapsed());
    let now = storage(&root);
    peak = (peak.0.max(now.0), peak.1.max(now.1));
    assert_eq!(port.file_cards().unwrap().value, single);
    let mut too_many = changed.to_vec();
    too_many.push(FileCardReference {
        name: String::new(),
        path: String::new(),
    });
    assert!(port
        .save_file_cards(large_version, too_many.into())
        .is_err());
    let too_big: Arc<[FileCardReference]> = vec![FileCardReference {
        name: String::new(),
        path: "p".repeat(MAX_CATALOGUE_FILE_BYTES - 22),
    }]
    .into();
    assert!(port.save_file_cards(large_version, too_big).is_err());
    assert!(port
        .save_root(Some(edits[1].0), Some(PathBuf::new()))
        .is_err());
    // Pin a real SQLite reader while two legal full-value edits dirty the WAL.
    assert_eq!(checkpoint(&service).0, 0);
    let reader = Connection::open(root.database_file()).unwrap();
    reader
        .execute_batch("BEGIN; SELECT count(*) FROM catalogue_head;")
        .unwrap();
    let mut replacement = single.to_vec();
    replacement[0].path.replace_range(..1, "q");
    let replacement: Arc<[FileCardReference]> = replacement.into();
    let next = port
        .save_file_cards(large_version, Arc::clone(&replacement))
        .unwrap();
    let last = port.save_file_cards(next, Arc::clone(&single)).unwrap();
    let held_storage = storage(&root);
    peak = (peak.0.max(held_storage.0), peak.1.max(held_storage.1));
    reader.execute_batch("ROLLBACK").unwrap();
    drop(reader);
    let checkpoint_result = checkpoint(&service);
    assert_eq!(checkpoint_result.0, 0);
    let after_checkpoint = storage(&root);
    assert_eq!(after_checkpoint.1, 0);
    // Failed writes cannot replace the last durable head.
    service.set_write_fault_for_test(Arc::new(|| true));
    let failed = port
        .save_file_cards(last, Arc::clone(&changed))
        .unwrap_err();
    assert_eq!(port.file_cards().unwrap().version, last);
    service.set_write_fault_for_test(Arc::new(|| false));
    let final_version = port.save_file_cards(last, Arc::clone(&changed)).unwrap();
    checkpoint(&service);
    let conn = Connection::open(root.database_file()).unwrap();
    let freed: i64 = conn
        .query_row("PRAGMA freelist_count", [], |row| row.get(0))
        .unwrap();
    let counts: (i64, i64, i64) = conn.query_row("SELECT (SELECT count(*) FROM catalogue_head),(SELECT count(*) FROM import_ledger WHERE domain='catalogue-references'),(SELECT max(length(body)) FROM catalogue_piece)", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
    assert_eq!((counts.0, counts.1), (3, 3));
    assert!(counts.2 <= 32 * 1024);
    drop(conn);
    drop(port);
    drop(service);
    let service = open_timed(&root, &owner);
    let port = service.catalogue_references();
    assert_eq!(port.file_cards().unwrap().version, final_version);
    assert_eq!(port.file_cards().unwrap().value, changed);
    drop(port);
    drop(service);
    // SAFETY: these diagnostic functions do not dereference caller pointers.
    let heap = unsafe {
        (
            rusqlite::ffi::sqlite3_memory_used(),
            rusqlite::ffi::sqlite3_memory_highwater(0),
        )
    };
    let owner = owner.lock().unwrap();
    let measured = owner
        .iter()
        .filter(|(class, _)| class.starts_with("catalogue "))
        .flat_map(|(_, times)| times)
        .copied()
        .fold(0.0_f64, f64::max);
    let pass = peak.0 <= sql::DB_TOTAL_PAGES
        && peak.1 <= sql::WAL_MAX_BYTES
        && heap.1 <= sql::SOFT_HEAP_LIMIT
        && measured <= occupancy_tests::THRESHOLD_MS;
    let report = json!({"verdict":if pass {"pass"} else {"fail"},
        "fixture":{"ordered_rows":MAX_CATALOGUE_FILE_ROWS,"canonical_source_bytes":source.len(),"maximum_single_body_source_bytes":MAX_CATALOGUE_FILE_BYTES,"root_bytes":MAX_CATALOGUE_ROOT_BYTES},
        "boundary":"reference metadata hydration and edits only; source compilation and transpile caches are excluded",
        "other_active_state":{"operator_loadout_rows":loadouts.len(),"preference_payload_bytes":100 * 1024},
        "contention":{"concurrent_independent_components":3,"stale_component_conflicts":3,"held_reader_edits":2,"failed_save_durable_unchanged":true,"failure":failed.to_string()},
        "sqlite":{"page_size":sql::PAGE_SIZE,"database_page_high_water":peak.0,"database_page_cap":sql::DB_TOTAL_PAGES,"earlier_domain_reservation_pages":sql::DB_MAX_PAGES,"wal_bytes_high_water":peak.1,"wal_bytes_cap":sql::WAL_MAX_BYTES,"wal_frame_cap":sql::WAL_MAX_FRAMES,"frames_at_high_water":peak.1.saturating_sub(32)/(sql::PAGE_SIZE+24),"import_pages":import_storage.0,"held_reader_wal_bytes":held_storage.1,"wal_after_checkpoint":after_checkpoint.1,"free_pages_after_shrink":freed,"maximum_chunk_bytes":counts.2,"heap_current":heap.0,"heap_high_water":heap.1,"heap_cap":sql::SOFT_HEAP_LIMIT},
        "owner_timings_ms":occupancy_tests::summarize(&owner),"residence_timings_ms":occupancy_tests::summarize(&residence),"whole_catalogue_owner_job_max_ms":measured,"owner_job_limit_ms":occupancy_tests::THRESHOLD_MS});
    std::fs::write(
        out.join("references-budget.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    std::fs::write(
        out.join("conversion.json"),
        serde_json::to_vec_pretty(&json!({"sources":receipts})).unwrap(),
    )
    .unwrap();
    assert!(
        pass,
        "reference qualification exceeded an unchanged shared cap: {report}"
    );
}
