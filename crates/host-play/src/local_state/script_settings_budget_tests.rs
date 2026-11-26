//! S10 release qualification on a disposable root. The real writer's hook
//! measures entire jobs, including conversion, commit, and file retirement.

use std::path::Path;
use std::time::Instant;

use api::local_state::{
    AccountKey, ContentScope, RuntimeSettingsEdit, MAX_RUNTIME_SETTINGS_ROWS,
    MAX_RUNTIME_SETTING_VALUE_BYTES,
};
use client::io::ClientRevision;
use script::ScriptSource::Catalog;
use serde_json::Value;

use super::*;

const MAX: usize = operator_settings::MAX_BYTES;
const PROFILE: &str = "local-289";
const CONTENT: &str = "test-content";

fn account(index: usize) -> String {
    format!("alice{index:03}")
}

fn identity(index: usize) -> DomainIdentity {
    DomainIdentity::new(
        LocalStateDomain::ScriptRuntimeSettings,
        AccountKey::try_new(PROFILE, &account(index)).unwrap(),
        ContentScope::try_new(ClientRevision::R289, CONTENT).unwrap(),
    )
}

fn op_fixture(cards: usize) -> Vec<u8> {
    let empty = || Value::Object(Default::default());
    let mut map = serde_json::Map::new();
    for index in 0..cards {
        map.insert(format!("catalog:Card{index:04}"), empty());
    }
    if cards == 1 {
        map.insert("catalog:Card0000".into(), json!({"bulk":""}));
        let size = serde_json::to_vec(&map).unwrap().len();
        map.insert(
            "catalog:Card0000".into(),
            json!({"bulk":"o".repeat(MAX - size)}),
        );
    } else {
        for bag in map.values_mut() {
            *bag = json!({"bulk":"o".repeat(90)});
        }
        let size = serde_json::to_vec(&map).unwrap().len();
        assert!(size <= MAX);
        map.insert(
            "catalog:Card0000".into(),
            json!({"bulk":"o".repeat(90 + MAX - size)}),
        );
    }
    let bytes = serde_json::to_vec(&map).unwrap();
    assert_eq!(bytes.len(), MAX);
    bytes
}

fn runtime_fixture(index: usize, rows: usize) -> Vec<u8> {
    let mut doc = json!({
        "schema_version": 1, "profile": PROFILE, "account": account(index),
        "revision": 289, "content": CONTENT,
        "rows": (0..rows).map(|key| json!({"namespace":"Box","key":format!("k{key:04}"),"raw":""})).collect::<Vec<_>>()
    });
    let mut remaining = MAX - serde_json::to_vec_pretty(&doc).unwrap().len();
    for row in doc["rows"].as_array_mut().unwrap() {
        let len = remaining.min(MAX_RUNTIME_SETTING_VALUE_BYTES);
        row["raw"] = Value::String("r".repeat(len));
        remaining -= len;
    }
    assert_eq!(remaining, 0);
    let bytes = serde_json::to_vec_pretty(&doc).unwrap();
    assert_eq!(bytes.len(), MAX);
    bytes
}

fn runtime_path(root: &StateRoot, index: usize) -> std::path::PathBuf {
    root.script_runtime_settings_file(PROFILE, &account(index), 289, CONTENT)
}

fn edit(identity: &DomainIdentity, version: StoreVersion, key: &str) -> DomainEdit {
    DomainEdit::ScriptRuntimeSettings(
        RuntimeSettingsEdit::try_set(identity.clone(), Some(version), "Box", key, "raw").unwrap(),
    )
}

fn elapsed(log: &mut occupancy_tests::Samples, name: &str, started: Instant) {
    occupancy_tests::record(log, name, started.elapsed());
}

fn job_samples(log: &mut occupancy_tests::Samples, timeline: &Timeline) {
    for (class, ms) in timeline.take() {
        log.entry(class.to_string()).or_default().push(ms);
    }
}

fn emit(dir: &Path, name: &str, doc: Value) {
    std::fs::write(dir.join(name), serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
}

fn close(service: Arc<LocalStateService>, log: &mut occupancy_tests::Samples) {
    let started = Instant::now();
    drop(service);
    elapsed(log, "close", started);
}

fn storage(root: &StateRoot) -> (u64, u64) {
    let conn = inspect(root);
    let pages: i64 = conn
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .unwrap();
    (u64::try_from(pages).unwrap(), wal_bytes(root))
}

fn conversions(root: &StateRoot) -> Vec<Value> {
    let conn = inspect(root);
    let mut statement = conn
        .prepare(
            "SELECT domain, source, kind, evidence, path FROM import_ledger
         WHERE domain IN ('script-settings','script-runtime-settings') AND digest IS NOT NULL
         ORDER BY domain, source",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            let path: String = row.get(4)?;
            Ok(json!({
                "domain": row.get::<_, String>(0)?, "identity": row.get::<_, String>(1)?,
                "moment": "first hydration", "kind": row.get::<_, String>(2)?,
                "evidence": row.get::<_, String>(3)?,
                "filename": Path::new(&path).file_name().and_then(|name| name.to_str()),
            }))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
#[ignore = "release-only S10 qualification; run alone with S10_OUT set"]
fn script_settings_release_budgets() {
    let dir = std::path::PathBuf::from(std::env::var_os("S10_OUT").expect("S10_OUT"));
    let scratch = IsolatedEnv::enter("s10-budget");
    let root = root_of(&scratch);
    let mut jobs = occupancy_tests::Samples::new();
    let mut waits = occupancy_tests::Samples::new();
    let mut peak_pages = 0;
    let mut peak_wal = 0;
    let mut measure_storage = || {
        let (pages, wal) = storage(&root);
        peak_pages = peak_pages.max(pages);
        peak_wal = peak_wal.max(wal);
    };

    let pref = root.preferences_file();
    write_private(&pref, &d_document(0));
    write_private(
        &root.path().join("script-settings.json"),
        &op_fixture(operator_settings::MAX_CARDS),
    );
    let (service, timeline) = Timeline::open_timed(&root);
    let preferences = service.preferences();
    let _ = ready(&preferences);
    reset_sqlite_highwater();
    let operator = service.operator_script_settings();
    let started = Instant::now();
    operator.wait_ready(WAIT).unwrap();
    elapsed(&mut waits, "operator-first-import", started);
    let started = Instant::now();
    operator
        .set_value(Catalog, "Card0000", "bulk", json!(false))
        .unwrap()
        .wait(WAIT)
        .unwrap();
    elapsed(&mut waits, "operator-edit-residence", started);
    let bank = submit_hint(&service, "alice", 1_760_000_000);
    assert!(matches!(
        service.wait_durable(bank, WAIT),
        Durability::Durable(_)
    ));
    service
        .readback_lease(&bank_identity("alice"), WAIT)
        .unwrap()
        .unwrap();
    save_wide(&preferences, 1);

    let full = identity(0);
    let full_bytes = runtime_fixture(0, MAX_RUNTIME_SETTINGS_ROWS);
    write_private(&runtime_path(&root, 0), &full_bytes);
    let started = Instant::now();
    let baseline = service.readback_lease(&full, WAIT).unwrap().unwrap();
    elapsed(&mut waits, "runtime-first-import", started);
    let import_version = baseline.version();
    assert_eq!(
        baseline
            .snapshot()
            .as_script_runtime_settings()
            .unwrap()
            .len(),
        MAX_RUNTIME_SETTINGS_ROWS
    );
    let first = edit(&full, import_version, "k0000");
    let second = edit(&full, import_version, "k0001");
    let started = Instant::now();
    let SubmitReceipt::Accepted { request } = service.submit(first) else {
        panic!("first edit not admitted")
    };
    assert!(matches!(
        service.wait_durable(request, WAIT),
        Durability::Durable(_)
    ));
    elapsed(&mut waits, "runtime-edit-residence", started);
    match service.submit(second) {
        SubmitReceipt::Refused(ErrorCode::Conflict { .. }) => {}
        SubmitReceipt::Accepted { request } => assert!(matches!(
            service.wait_durable(request, WAIT),
            Durability::NotDurable(Settlement::Failed(ErrorCode::Conflict { .. }))
        )),
        receipt => panic!("unexpected stale-edit receipt: {receipt:?}"),
    }

    for index in 1..MAX_SCRIPT_RUNTIME_SETTINGS_STORES {
        service.readback(&identity(index), WAIT).unwrap();
    }
    let resident_count = {
        let mut state = service.shared.state.lock();
        let keys: Vec<_> = (0..MAX_SCRIPT_RUNTIME_SETTINGS_STORES)
            .map(|index| service.store_key(&identity(index)).unwrap())
            .collect();
        assert_eq!(
            state
                .stores
                .keys()
                .filter(|key| key.domain() == LocalStateDomain::ScriptRuntimeSettings)
                .count(),
            MAX_SCRIPT_RUNTIME_SETTINGS_STORES,
        );
        assert!(keys.iter().all(|key| state.stores.contains_key(key)));
        // The pending readbacks pin their stores. Hold the scheduler lock
        // across the batch so the writer cannot finish any before admission.
        for key in &keys {
            assert!(service.shared.request_readback(&mut state, key).is_some());
        }
        let extra = service.store_key(&identity(keys.len())).unwrap();
        assert!(matches!(state.admit(&extra), Err(ErrorCode::Busy { .. })));
        keys.len()
    };
    let (resident_heap, _) = sqlite_heap();
    service
        .readback(&identity(MAX_SCRIPT_RUNTIME_SETTINGS_STORES - 1), WAIT)
        .unwrap();
    drop(baseline);

    let pinned_reader = pin_reader(&root);
    let mut max_stores = 0;
    let mut refused_bytes = false;
    for index in 256..=273 {
        let path = runtime_path(&root, index);
        let bytes = runtime_fixture(index, 17);
        write_private(&path, &bytes);
        measure_storage();
        match service.readback_lease(&identity(index), WAIT) {
            Ok(Some(lease)) => {
                assert_eq!(
                    lease.snapshot().as_script_runtime_settings().unwrap().len(),
                    17
                );
                max_stores += 1;
            }
            Err(ErrorCode::Failed { .. }) => {
                assert_eq!(std::fs::read(&path).unwrap(), bytes);
                refused_bytes = true;
                break;
            }
            _ => panic!("unexpected capacity result for {}", account(index)),
        }
    }
    assert!(refused_bytes && max_stores > 0);
    measure_storage();
    let (current, peak) = sqlite_heap();
    drop(pinned_reader);
    let wal_before_close = wal_bytes(&root);
    drop(preferences);
    drop(operator);
    close(service, &mut jobs);
    job_samples(&mut jobs, &timeline);
    let (service, timeline) = Timeline::open_timed(&root);
    let operator = service.operator_script_settings();
    let started = Instant::now();
    operator.wait_ready(WAIT).unwrap();
    assert_eq!(operator.overrides(Catalog, "Card0000")["bulk"], false);
    elapsed(&mut waits, "operator-restart-cold-read", started);
    let started = Instant::now();
    let lease = service.readback_lease(&full, WAIT).unwrap().unwrap();
    assert_eq!(
        lease.snapshot().as_script_runtime_settings().unwrap()[0].raw(),
        "raw"
    );
    elapsed(&mut waits, "runtime-restart-cold-read", started);
    let restart_version = lease.version();
    drop(operator);
    close(service, &mut jobs);
    job_samples(&mut jobs, &timeline);

    let single = phase_root(&scratch, "single-card");
    write_private(&single.path().join("script-settings.json"), &op_fixture(1));
    let (service, timeline) = Timeline::open_timed(&single);
    let operator = service.operator_script_settings();
    operator.wait_ready(WAIT).unwrap();
    let value = operator.card(Catalog, "Card0000").unwrap();
    let width = value["bulk"].as_str().unwrap().len();
    let started = Instant::now();
    operator
        .set_value(Catalog, "Card0000", "bulk", json!("p".repeat(width)))
        .unwrap()
        .wait(WAIT)
        .unwrap();
    elapsed(&mut waits, "largest-single-bag-edit-residence", started);
    let value = operator.card(Catalog, "Card0000").unwrap();
    assert_eq!(value["bulk"].as_str().unwrap().len(), width);
    assert!(value["bulk"].as_str().unwrap().starts_with('p'));
    drop(operator);
    close(service, &mut jobs);
    job_samples(&mut jobs, &timeline);

    let worst = occupancy_tests::worst_ms(&jobs);
    let (heap_after_close, final_peak) = sqlite_heap();
    let heap_peak = peak.max(final_peak);
    let verdict = if worst < occupancy_tests::THRESHOLD_MS
        && heap_peak < 8 * 1024 * 1024
        && peak_pages <= sql::DB_MAX_PAGES
        && peak_wal <= sql::WAL_MAX_BYTES
    {
        "pass"
    } else {
        "fail"
    };
    let combined = json!({
        "jobs": occupancy_tests::summarize(&jobs), "caller_waits": occupancy_tests::summarize(&waits),
        "peak_pages": peak_pages, "page_cap": sql::DB_MAX_PAGES,
        "peak_wal_bytes": peak_wal, "wal_cap_bytes": sql::WAL_MAX_BYTES,
        "wal_bytes_after_reader_release": wal_before_close,
        "sqlite_c_heap_current_bytes": current, "sqlite_c_heap_peak_bytes": heap_peak,
        "sqlite_c_heap_after_close_bytes": heap_after_close, "sqlite_c_heap_limit_bytes": 8 * 1024 * 1024,
        "max_job_ms": worst, "job_limit_ms": occupancy_tests::THRESHOLD_MS,
        "verdict": verdict,
    });
    emit(&dir, "combined-budget.json", combined.clone());
    let fixture_census: Value =
        serde_json::from_slice(&std::fs::read(dir.join("script-settings-fixtures.json")).unwrap())
            .unwrap();
    emit(
        &dir,
        "script-settings-budget.json",
        json!({
            "max_encoded_map_bytes": MAX, "admitted_card_keys": operator_settings::MAX_CARDS,
            "max_single_bag_fixture_encoded_map_bytes": MAX,
            "supported_fixtures": fixture_census, "combined": combined,
        }),
    );
    emit(
        &dir,
        "runtime-settings-budget.json",
        json!({
            "max_encoded_store_bytes": MAX, "max_rows": MAX_RUNTIME_SETTINGS_ROWS,
            "resident_cache_entries_pinned": resident_count, "refused_another_resident": true,
            "c_heap_bytes_with_256_residents": resident_heap,
            "additional_max_size_scopes_admitted_with_other_domains": max_stores,
            "first_refused_max_size_scope_preserved_source": refused_bytes,
            "stale_expected_version_refused": true, "combined": combined,
        }),
    );
    let operator_version: Vec<u8> = inspect(&root)
        .query_row(
            "SELECT version FROM operator_script_head WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    emit(
        &dir,
        "conversion.json",
        json!({
            "receipts": conversions(&root), "max_operator_cards": operator_settings::MAX_CARDS,
            "max_runtime_rows": MAX_RUNTIME_SETTINGS_ROWS,
            "runtime_first_version": import_version.get(), "runtime_restart_version": restart_version.get(),
            "operator_restart_version": sql::decode_u64(&operator_version).unwrap(),
            "restart_readback_verified": true, "test": "script_settings_release_budgets",
        }),
    );
    assert_eq!(verdict, "pass", "{combined}");
    println!("S10 settings qualification: {verdict}");
}
