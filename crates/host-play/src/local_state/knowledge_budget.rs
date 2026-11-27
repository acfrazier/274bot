use std::io::{BufRead, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use api::identity::{DurableTimestamp, RequestId, Settlement};
use api::local_state::{
    validate_knowledge_payload, AccountKey, BankHint, ContentScope, DomainEdit, DomainIdentity,
    ErrorCode, KnowledgeKey, KnowledgeProvenance, KnowledgeRecord, KnowledgeRevocation,
    KnowledgeStatus, LocalStateDomain, LocalStatePort, StoreVersion, SubmitReceipt,
    ValidationReason, KNOWLEDGE_SCHEMA_VERSION, MAX_KNOWLEDGE_BYTES, MAX_KNOWLEDGE_RECORDS,
};
use client::io::ClientRevision;
use rusqlite::{Connection, OpenFlags};
use script::IsolatedEnv;
use serde_json::{json, Value};

use super::{
    occupancy_tests, sql, KnowledgeFact, KnowledgeSnapshot, LocalStateService, RequestState,
    StateRoot, MAX_KNOWLEDGE_STORES,
};

const WAIT: Duration = Duration::from_secs(60);
const PROFILE: &str = "local-289";
const CONTENT: &str = "test-content";
const QUEUED_WRITES: usize = 8;
const ACTIVE_WRITES: usize = 1;
const ROUTE: [u8; 6] = [5, 4, 5, 15, 16, 12];

fn identity(index: usize) -> DomainIdentity {
    DomainIdentity::new(
        LocalStateDomain::Knowledge,
        AccountKey::try_new(PROFILE, &format!("alice{index:03}")).unwrap(),
        ContentScope::try_new(ClientRevision::R289, CONTENT).unwrap(),
    )
}

fn root_of(scratch: &IsolatedEnv) -> StateRoot {
    StateRoot::new(scratch.home.join(".274bot"))
}

fn elapsed_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn record_residence(samples: &mut occupancy_tests::Samples, class: &str, started: Instant) {
    occupancy_tests::record(samples, class, started.elapsed());
}

fn make_record(
    identity: &DomainIdentity,
    name: &str,
    expected: StoreVersion,
    status: KnowledgeStatus,
    bytes: &[u8],
    observed_at: u64,
    validation_ms: &mut Vec<f64>,
) -> KnowledgeRecord {
    let started = Instant::now();
    let payload = validate_knowledge_payload(KNOWLEDGE_SCHEMA_VERSION, bytes).unwrap();
    let record = KnowledgeRecord::try_new(
        identity.account().clone(),
        identity.scope().clone(),
        expected,
        name,
        KnowledgeProvenance::Observed,
        status,
        payload,
        DurableTimestamp::from_unix_seconds(observed_at),
    )
    .unwrap();
    validation_ms.push(elapsed_ms(started.elapsed()));
    record
}

fn resolve(
    service: &LocalStateService,
    receipt: SubmitReceipt,
) -> Result<api::local_state::DurableReceipt, ErrorCode> {
    let request = match receipt {
        SubmitReceipt::Accepted { request } => request,
        SubmitReceipt::Refused(code) => return Err(code),
    };
    match service.wait_request(request, WAIT) {
        RequestState::Settled(Settlement::Completed(receipt)) => Ok(receipt),
        RequestState::Settled(Settlement::Failed(error)) => Err(error),
        other => panic!("knowledge request did not settle: {other:?}"),
    }
}

fn readback(
    service: &LocalStateService,
    identity: &DomainIdentity,
    waits: &mut occupancy_tests::Samples,
) -> Result<KnowledgeSnapshot, ErrorCode> {
    let started = Instant::now();
    let result = service
        .start_knowledge_readback(identity)
        .and_then(|ticket| service.wait_knowledge_readback(identity, ticket, WAIT));
    record_residence(waits, "knowledge-readback-residence", started);
    result
}

fn publish(
    service: &LocalStateService,
    record: KnowledgeRecord,
    waits: &mut occupancy_tests::Samples,
) -> Result<api::local_state::DurableReceipt, ErrorCode> {
    let started = Instant::now();
    let result = resolve(service, service.publish_knowledge(record));
    record_residence(waits, "knowledge-write-residence", started);
    result
}
fn publish_accepted(
    service: &LocalStateService,
    record: KnowledgeRecord,
    waits: &mut occupancy_tests::Samples,
) -> Result<api::local_state::DurableReceipt, ErrorCode> {
    let started = Instant::now();
    let request = match service.publish_knowledge(record) {
        SubmitReceipt::Accepted { request } => request,
        SubmitReceipt::Refused(error) => {
            record_residence(waits, "knowledge-write-residence", started);
            panic!("knowledge page-limit write was refused before acceptance: {error:?}");
        }
    };
    let result = match service.wait_request(request, WAIT) {
        RequestState::Settled(Settlement::Completed(receipt)) => Ok(receipt),
        RequestState::Settled(Settlement::Failed(error)) => Err(error),
        other => panic!("accepted knowledge write did not settle: {other:?}"),
    };
    record_residence(waits, "knowledge-write-residence", started);
    result
}

fn inspect(root: &StateRoot) -> Connection {
    let connection = Connection::open_with_flags(
        root.database_file(),
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    connection.busy_timeout(WAIT).unwrap();
    connection
}

fn database_sequence(root: &StateRoot) -> u64 {
    let connection = inspect(root);
    let encoded: Vec<u8> = connection
        .query_row("SELECT sequence FROM store_meta WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    sql::decode_u64(&encoded).unwrap()
}

#[derive(Debug, Clone, Copy)]
struct DurableCounts {
    records: usize,
    identities: usize,
    payload_bytes: usize,
}

fn durable_counts(root: &StateRoot) -> DurableCounts {
    let connection = inspect(root);
    let (records, payload_bytes): (i64, i64) = connection
        .query_row(
            "SELECT count(*), coalesce(sum(length(payload)), 0) FROM knowledge_fact",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let identities: i64 = connection
        .query_row(
            "SELECT count(*) FROM (
                 SELECT profile, account, revision, content
                 FROM knowledge_fact
                 GROUP BY profile, account, revision, content
             )",
            [],
            |row| row.get(0),
        )
        .unwrap();
    DurableCounts {
        records: usize::try_from(records).unwrap(),
        identities: usize::try_from(identities).unwrap(),
        payload_bytes: usize::try_from(payload_bytes).unwrap(),
    }
}

fn wal_bytes(root: &StateRoot) -> u64 {
    let [_, wal, _] = sql::database_set(root);
    std::fs::metadata(wal).map_or(0, |metadata| metadata.len())
}

fn storage(root: &StateRoot) -> (u64, u64) {
    let pages: i64 = inspect(root)
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .unwrap();
    (u64::try_from(pages).unwrap(), wal_bytes(root))
}

fn record_storage_peak(root: &StateRoot, peak: &mut (u64, u64)) -> (u64, u64) {
    let current = storage(root);
    peak.0 = peak.0.max(current.0);
    peak.1 = peak.1.max(current.1);
    current
}

fn sqlite_heap() -> (i64, i64) {
    // SAFETY: the bundled SQLite library is initialized by the open state database.
    // These calls read process-wide counters and do not dereference pointers.
    unsafe {
        (
            rusqlite::ffi::sqlite3_memory_used(),
            rusqlite::ffi::sqlite3_memory_highwater(0),
        )
    }
}

fn reset_sqlite_highwater() {
    // SAFETY: the bundled SQLite library is initialized. The flag only resets its counter.
    unsafe {
        rusqlite::ffi::sqlite3_memory_highwater(1);
    }
}

type JobEvents = Arc<Mutex<Vec<(&'static str, f64)>>>;

fn open_timed(root: &StateRoot) -> (Arc<LocalStateService>, JobEvents) {
    let events = JobEvents::default();
    let sink = Arc::clone(&events);
    let service = LocalStateService::open_process_timed_for_test(
        root.clone(),
        Arc::new(move |class, took| {
            sink.lock().unwrap().push((class, elapsed_ms(took)));
        }),
    );
    (service, events)
}

fn queue_write(
    identity: &DomainIdentity,
    slot: usize,
    validation_ms: &mut Vec<f64>,
) -> KnowledgeRecord {
    let name = format!("queue.fact_{slot:02}");
    let bytes = vec![b'q'; MAX_KNOWLEDGE_BYTES];
    make_record(
        identity,
        &name,
        StoreVersion::INITIAL,
        KnowledgeStatus::Known,
        &bytes,
        1_760_000_100 + slot as u64,
        validation_ms,
    )
}

#[derive(Debug, Clone, Copy)]
struct PendingWrite {
    request: RequestId,
    submitted: Instant,
}

fn exercise_queue_bound(
    root: &StateRoot,
    validation_ms: &mut Vec<f64>,
    waits: &mut occupancy_tests::Samples,
) -> Value {
    let service = LocalStateService::open_process(root.clone());
    let identity = identity(0);
    drop(readback(&service, &identity, waits).unwrap());

    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let released = Mutex::new(release_rx);
    let first_stall = AtomicBool::new(false);
    service.set_sql_seams_for_test(sql::Seams {
        stall_in_sql: Some(Arc::new(move || {
            if !first_stall.swap(true, Ordering::SeqCst) {
                let _ = entered_tx.send(());
                let _ = released.lock().unwrap().recv_timeout(WAIT);
            }
        })),
        ..sql::Seams::default()
    });

    let mut pending = Vec::with_capacity(QUEUED_WRITES + ACTIVE_WRITES + 1);
    let mut enqueue_error = None;
    for slot in 0..QUEUED_WRITES + ACTIVE_WRITES {
        let record = queue_write(&identity, slot, validation_ms);
        let submitted = Instant::now();
        match service.publish_knowledge(record) {
            SubmitReceipt::Accepted { request } => {
                pending.push(PendingWrite { request, submitted });
                if slot == 0 && entered_rx.recv_timeout(WAIT).is_err() {
                    let _ = release_tx.send(());
                    panic!("the active knowledge write did not enter SQL");
                }
            }
            SubmitReceipt::Refused(error) => {
                enqueue_error = Some(error);
                break;
            }
        }
    }

    let overflow_name = "queue.fact_overflow";
    let overflow = if enqueue_error.is_none() {
        let record = make_record(
            &identity,
            overflow_name,
            StoreVersion::INITIAL,
            KnowledgeStatus::Known,
            b"overflow",
            1_760_000_200,
            validation_ms,
        );
        let submitted = Instant::now();
        match service.publish_knowledge(record) {
            SubmitReceipt::Refused(error) => Some(error),
            SubmitReceipt::Accepted { request } => {
                pending.push(PendingWrite { request, submitted });
                None
            }
        }
    } else {
        None
    };

    release_tx
        .send(())
        .expect("the active write is waiting for release");
    for write in &pending {
        match service.wait_request(write.request, WAIT) {
            RequestState::Settled(Settlement::Completed(_)) => occupancy_tests::record(
                waits,
                "knowledge-queue-residence",
                write.submitted.elapsed(),
            ),
            other => panic!("queued knowledge write did not complete: {other:?}"),
        }
    }

    let snapshot = service.knowledge_snapshot(&identity).unwrap();
    let stored_records = snapshot.records().len();
    drop(snapshot);
    drop(service);

    assert_eq!(
        enqueue_error, None,
        "all nine writer slots must be accepted"
    );
    assert_eq!(pending.len(), QUEUED_WRITES + ACTIVE_WRITES);
    assert_eq!(
        overflow,
        Some(ErrorCode::Busy {
            domain: LocalStateDomain::Knowledge,
        })
    );
    assert_eq!(stored_records, QUEUED_WRITES + ACTIVE_WRITES);

    json!({
        "queued_limit": QUEUED_WRITES,
        "active_limit": ACTIVE_WRITES,
        "accepted_active_plus_queued": pending.len(),
        "overflow_refusal": format!("{overflow:?}"),
        "durable_queue_records": stored_records,
        "staged_payload_bytes": pending.len() * MAX_KNOWLEDGE_BYTES,
        "staged_payload_limit_bytes": (QUEUED_WRITES + ACTIVE_WRITES) * MAX_KNOWLEDGE_BYTES,
    })
}

fn knowledge_jobs(events: &[(&'static str, f64)]) -> occupancy_tests::Samples {
    let mut samples = occupancy_tests::Samples::new();
    for (class, took_ms) in events {
        if matches!(
            *class,
            "knowledge-readback" | "knowledge-write" | "knowledge-invalidate"
        ) {
            samples
                .entry((*class).to_string())
                .or_default()
                .push(*took_ms);
        }
    }
    samples
}

fn max_sample(values: &[f64]) -> f64 {
    values.iter().copied().fold(0.0, f64::max)
}

fn cache_usage(snapshots: &[KnowledgeSnapshot]) -> (usize, usize, usize, usize) {
    let mut records = 0;
    let mut payload_bytes = 0;
    let mut max_records = 0;
    for snapshot in snapshots {
        max_records = max_records.max(snapshot.records().len());
        records += snapshot.records().len();
        payload_bytes += snapshot
            .records()
            .iter()
            .map(|fact| fact.record().payload().len())
            .sum::<usize>();
    }
    (records, payload_bytes, max_records, snapshots.len())
}

fn fact_payload<'a>(snapshot: &'a KnowledgeSnapshot, key: &KnowledgeKey) -> Option<&'a [u8]> {
    snapshot
        .fact(key)
        .map(|fact: &KnowledgeFact| fact.record().payload())
}

fn seed_preferences(root: &StateRoot) -> usize {
    let bulk: serde_json::Map<String, Value> = (0..250)
        .map(|member| {
            (
                format!("m{member:03}"),
                json!(format!("seed-{}", "d".repeat(3_900))),
            )
        })
        .collect();
    let bytes = serde_json::to_vec_pretty(&json!({ "bulk": bulk })).unwrap();
    assert!(bytes.len() as u64 <= sql::DOCUMENT_MAX);
    vault::create_private_dir(root.path()).unwrap();
    vault::write_private_file(&root.preferences_file(), &bytes).unwrap();
    bytes.len()
}

fn activate_other_domains(service: &Arc<LocalStateService>) {
    assert_eq!(
        service.preferences().reload().unwrap()["bulk"]
            .as_object()
            .unwrap()
            .len(),
        250
    );
    let owner = identity(0);
    let bank = DomainIdentity::new(
        LocalStateDomain::BankHints,
        owner.account().clone(),
        owner.scope().clone(),
    );
    drop(service.readback(&bank, WAIT).unwrap());
    let hint = BankHint::try_new(
        bank,
        None,
        vec![(995, 10)],
        DurableTimestamp::from_unix_seconds(1_760_000_200),
    )
    .unwrap();
    resolve(service, service.submit(DomainEdit::Bank(hint))).unwrap();
    let key = api::local_state::LoadoutKey::OperatorPresets;
    let ticket = service.start_loadout_readback(&key).unwrap();
    let snapshot = service.wait_loadout_readback(&key, ticket, WAIT).unwrap();
    let edit = api::local_state::LoadoutEdit::upsert(
        DomainIdentity::new(
            LocalStateDomain::LoadoutsReferences,
            AccountKey::try_new("operator", "operator").unwrap(),
            ContentScope::try_new(ClientRevision::R289, "operator").unwrap(),
        ),
        Some(snapshot.version()),
        api::loadout::Loadout::new("budget"),
    )
    .unwrap();
    resolve(service, service.submit(DomainEdit::Loadout(edit))).unwrap();
}

struct HeldReader(Option<Child>);

impl HeldReader {
    fn start(root: &StateRoot) -> Self {
        let mut reader = Self(Some(
            crate::test_process::spawn(
                std::process::Command::new(crate::test_process::executable())
                    .args([
                        "--ignored",
                        "--exact",
                        "local_state::knowledge_budget::knowledge_budget_reader_child",
                        "--nocapture",
                    ])
                    .env("KNOWLEDGE_READER_ROOT", root.path())
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped()),
            )
            .unwrap(),
        ));
        let mut output = std::io::BufReader::new(reader.0.as_mut().unwrap().stdout.take().unwrap());
        let mut ready = false;
        for _ in 0..10 {
            let mut line = String::new();
            if output.read_line(&mut line).unwrap() == 0 {
                break;
            }
            if line.contains("KNOWLEDGE_READER_READY") {
                ready = true;
                break;
            }
        }
        assert!(
            ready,
            "the second process did not pin a SQL read transaction"
        );
        reader.0.as_mut().unwrap().stdout = Some(output.into_inner());
        reader
    }

    fn finish(mut self) {
        let mut child = self.0.take().unwrap();
        child.stdin.take().unwrap().write_all(b"x").unwrap();
        assert!(child.wait().unwrap().success());
    }
}

impl Drop for HeldReader {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            if let Some(mut input) = child.stdin.take() {
                let _ = input.write_all(b"x");
            }
            let _ = child.wait();
        }
    }
}

#[test]
#[ignore = "second-process reader for the knowledge budget"]
fn knowledge_budget_reader_child() {
    let root = StateRoot::new(PathBuf::from(
        std::env::var_os("KNOWLEDGE_READER_ROOT").unwrap(),
    ));
    let connection = inspect(&root);
    connection.execute_batch("BEGIN;").unwrap();
    let _: Vec<u8> = connection
        .query_row("SELECT sequence FROM store_meta WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    println!("KNOWLEDGE_READER_READY");
    std::io::stdout().flush().unwrap();
    std::io::stdin().read_exact(&mut [0]).unwrap();
    connection.execute_batch("ROLLBACK;").unwrap();
}

#[test]
#[ignore = "release-only S7 knowledge qualification; run with KNOWLEDGE_BUDGET_OUT set"]
fn knowledge_release_budget() {
    let out = PathBuf::from(
        std::env::var_os("KNOWLEDGE_BUDGET_OUT").expect("KNOWLEDGE_BUDGET_OUT is set"),
    );
    let scratch = IsolatedEnv::enter("knowledge-release-budget");
    let root = root_of(&scratch);
    let preferences_bytes = seed_preferences(&root);
    let mut validation_ms = Vec::new();
    let mut waits = occupancy_tests::Samples::new();
    let queue = exercise_queue_bound(&root, &mut validation_ms, &mut waits);

    reset_sqlite_highwater();
    let (service, job_events) = open_timed(&root);
    activate_other_domains(&service);
    let held_reader = HeldReader::start(&root);
    let identities: Vec<_> = (0..MAX_KNOWLEDGE_STORES).map(identity).collect();
    let mut snapshots = Vec::with_capacity(MAX_KNOWLEDGE_STORES);
    for identity in &identities {
        snapshots.push(readback(&service, identity, &mut waits).unwrap());
    }

    let extra_identity = identity(MAX_KNOWLEDGE_STORES);
    let extra_error = readback(&service, &extra_identity, &mut waits).unwrap_err();
    assert_eq!(
        extra_error,
        ErrorCode::Busy {
            domain: LocalStateDomain::Knowledge,
        }
    );

    let route_key = KnowledgeKey::upass_grid_route();
    let route_json = serde_json::to_vec(&ROUTE).unwrap();
    assert_eq!(route_json.len(), 16);

    let mut peak_storage = (0, 0);
    for (identity, snapshot) in identities.iter().zip(&mut snapshots) {
        let record = make_record(
            identity,
            route_key.as_str(),
            snapshot.version(&route_key),
            KnowledgeStatus::Known,
            &ROUTE,
            1_760_000_300,
            &mut validation_ms,
        );
        let receipt = publish(&service, record, &mut waits).unwrap();
        let current = service.knowledge_snapshot(identity).unwrap();
        assert_eq!(fact_payload(&current, &route_key), Some(ROUTE.as_slice()));
        assert!(receipt.version() > StoreVersion::INITIAL);
        *snapshot = current;
        let last_storage = record_storage_peak(&root, &mut peak_storage);
        assert!(
            last_storage.0 <= sql::DB_MAX_PAGES,
            "knowledge writes exceeded the unchanged database page cap"
        );
        assert!(
            last_storage.1 <= sql::WAL_MAX_BYTES,
            "knowledge writes exceeded the unchanged WAL byte cap"
        );
    }
    assert!(snapshots
        .iter()
        .all(|snapshot| snapshot.fact(&route_key).is_some()));

    let invalidation_started = Instant::now();
    let route_storage = record_storage_peak(&root, &mut peak_storage);
    let route_heap = sqlite_heap();
    let invalidate = service.invalidate_knowledge(
        &identities[0],
        route_key.clone(),
        snapshots[0].version(&route_key),
        DurableTimestamp::from_unix_seconds(1_760_000_300),
        KnowledgeProvenance::Observed,
        KnowledgeRevocation::ObservationRefused,
    );
    let invalidation = resolve(&service, invalidate).unwrap();
    record_residence(
        &mut waits,
        "knowledge-invalidate-residence",
        invalidation_started,
    );
    let invalidated = service.knowledge_snapshot(&identities[0]).unwrap();
    assert_eq!(
        invalidated.fact(&route_key).unwrap().record().revocation(),
        Some(KnowledgeRevocation::ObservationRefused)
    );
    snapshots[0] = invalidated;
    let mut last_storage = record_storage_peak(&root, &mut peak_storage);
    held_reader.finish();

    let mut durable_rows = durable_counts(&root).records;
    let mut current_sequence = database_sequence(&root);
    let mut page_refusal = None;
    let mut general_records = 0usize;
    let mut general_payload_bytes = 0usize;
    'identities: for (identity_index, (identity, snapshot)) in
        identities.iter().zip(&mut snapshots).enumerate()
    {
        while snapshot.records().len() < MAX_KNOWLEDGE_RECORDS {
            let slot = snapshot.records().len();
            let name = format!("envelope.fact_{slot:02}");
            let typed_key = KnowledgeKey::try_new(&name).unwrap();
            let bytes = vec![(identity_index as u8).wrapping_add(slot as u8); MAX_KNOWLEDGE_BYTES];
            let record = make_record(
                identity,
                &name,
                snapshot.version(&typed_key),
                KnowledgeStatus::Known,
                &bytes,
                1_760_000_400 + identity_index as u64,
                &mut validation_ms,
            );
            let rows_before = durable_rows;
            let sequence_before = current_sequence;
            let pages_before = last_storage.0;
            let wal_before = last_storage.1;
            match publish_accepted(&service, record, &mut waits) {
                Ok(receipt) => {
                    current_sequence = receipt.version().get();
                    durable_rows += 1;
                    general_records += 1;
                    general_payload_bytes += MAX_KNOWLEDGE_BYTES;
                    *snapshot = service.knowledge_snapshot(identity).unwrap();
                    last_storage = record_storage_peak(&root, &mut peak_storage);
                    assert!(
                        last_storage.0 <= sql::DB_MAX_PAGES,
                        "knowledge writes exceeded the unchanged database page cap"
                    );
                    assert!(
                        last_storage.1 <= sql::WAL_MAX_BYTES,
                        "knowledge writes exceeded the unchanged WAL byte cap"
                    );
                }
                Err(error) => {
                    let after_storage = record_storage_peak(&root, &mut peak_storage);
                    let after_counts = durable_counts(&root);
                    let sequence_after = database_sequence(&root);
                    assert_eq!(
                        error,
                        ErrorCode::Validation {
                            domain: LocalStateDomain::Knowledge,
                            reason: ValidationReason::TooLong,
                        },
                        "the database page budget must reject the next 4 KiB fact"
                    );
                    assert_eq!(after_counts.records, rows_before);
                    assert_eq!(sequence_after, sequence_before);
                    assert_eq!(after_storage.0, pages_before);
                    assert!(after_storage.0 <= sql::DB_MAX_PAGES);
                    assert!(after_storage.1 <= sql::WAL_MAX_BYTES);
                    page_refusal = Some(json!({
                        "error": format!("{error:?}"),
                        "identity_index": identity_index,
                        "key": name,
                        "records_before": rows_before,
                        "records_after": after_counts.records,
                        "sequence_before": sequence_before,
                        "sequence_after": sequence_after,
                        "page_count_before": pages_before,
                        "page_count_after": after_storage.0,
                        "page_cap": sql::DB_MAX_PAGES,
                        "wal_bytes_before": wal_before,
                        "wal_bytes_after": after_storage.1,
                    }));
                    break 'identities;
                }
            }
        }
    }
    let page_refusal =
        page_refusal.expect("the physical page cap refuses before the full envelope");

    let durable = durable_counts(&root);
    assert_eq!(durable.records, durable_rows);
    assert_eq!(durable.identities, MAX_KNOWLEDGE_STORES);
    assert!(durable.records < MAX_KNOWLEDGE_RECORDS * MAX_KNOWLEDGE_STORES);
    let (cache_records, cache_payload_bytes, max_cache_records, pinned_identities) =
        cache_usage(&snapshots);
    assert_eq!(pinned_identities, MAX_KNOWLEDGE_STORES);
    assert!(max_cache_records <= MAX_KNOWLEDGE_RECORDS);
    assert_eq!(cache_records, durable.records);
    assert_eq!(cache_payload_bytes, durable.payload_bytes);

    let final_storage = record_storage_peak(&root, &mut peak_storage);
    let (sqlite_current_before_close, sqlite_peak) = sqlite_heap();
    drop(snapshots);
    drop(service);
    let final_storage_after_close = storage(&root);
    let (sqlite_current_after_close, sqlite_peak_after_close) = sqlite_heap();
    let sqlite_peak = sqlite_peak.max(sqlite_peak_after_close);

    let events = std::mem::take(&mut *job_events.lock().unwrap());
    let mut all_jobs = occupancy_tests::Samples::new();
    for (class, took_ms) in &events {
        all_jobs
            .entry((*class).to_string())
            .or_default()
            .push(*took_ms);
    }
    let measured_knowledge_jobs = knowledge_jobs(&events);
    for class in [
        "knowledge-readback",
        "knowledge-write",
        "knowledge-invalidate",
    ] {
        assert!(
            measured_knowledge_jobs
                .get(class)
                .is_some_and(|values| !values.is_empty()),
            "missing timed writer job class {class}"
        );
    }
    let max_knowledge_job_ms = measured_knowledge_jobs
        .values()
        .map(|values| max_sample(values))
        .fold(0.0, f64::max);
    let max_sql_job_ms = occupancy_tests::worst_ms(&all_jobs);
    let max_validation_ms = max_sample(&validation_ms);
    let db_cap_bytes = sql::DB_MAX_PAGES * sql::PAGE_SIZE;

    let heap_within_limit = sqlite_current_before_close < 8 * 1024 * 1024
        && sqlite_current_after_close < 8 * 1024 * 1024
        && sqlite_peak < 8 * 1024 * 1024;
    let page_within_limit = peak_storage.0 <= sql::DB_MAX_PAGES;
    let wal_within_limit = peak_storage.1 <= sql::WAL_MAX_BYTES;
    let timings_within_limit = max_sql_job_ms < occupancy_tests::THRESHOLD_MS
        && max_validation_ms < occupancy_tests::THRESHOLD_MS;
    let verdict =
        if heap_within_limit && page_within_limit && wal_within_limit && timings_within_limit {
            "pass"
        } else {
            "fail"
        };

    let report = json!({
        "verdict": verdict,
        "timings": {
            "validation": occupancy_tests::summary(&validation_ms),
            "knowledge_jobs": occupancy_tests::summarize(&measured_knowledge_jobs),
            "all_writer_jobs": occupancy_tests::summarize(&all_jobs),
            "caller_residence": occupancy_tests::summarize(&waits),
            "max_knowledge_job_ms": max_knowledge_job_ms,
            "max_sql_job_ms": max_sql_job_ms,
            "max_validation_ms": max_validation_ms,
            "limit_ms": occupancy_tests::THRESHOLD_MS,
        },
        "route": {
            "qualification": "This single-route result does not qualify a second or larger producer without future measured capacity approval.",
            "database_pages": route_storage.0,
            "wal_bytes_with_second_process_reader": route_storage.1,
            "sqlite_c_heap_current_bytes": route_heap.0,
            "sqlite_c_heap_highwater_bytes": route_heap.1,
            "other_active_domains": ["preferences", "bank-hints", "loadouts"],
            "preferences_source_bytes": preferences_bytes,
            "second_process_pinned_reader": true,
            "fixture_raw_bytes": ROUTE.len(),
            "fixture_json_bytes": route_json.len(),
            "resident_identities": MAX_KNOWLEDGE_STORES,
            "records_written": MAX_KNOWLEDGE_STORES,
            "raw_payload_bytes": ROUTE.len() * MAX_KNOWLEDGE_STORES,
            "json_payload_bytes": route_json.len() * MAX_KNOWLEDGE_STORES,
            "invalidated_identity": 0,
            "invalidation_version": invalidation.version().get(),
        },
        "general_envelope": {
            "records_per_identity_cap": MAX_KNOWLEDGE_RECORDS,
            "payload_bytes_per_record_cap": MAX_KNOWLEDGE_BYTES,
            "resident_identity_cap": MAX_KNOWLEDGE_STORES,
            "theoretical_records": MAX_KNOWLEDGE_RECORDS * MAX_KNOWLEDGE_STORES,
            "theoretical_payload_bytes": MAX_KNOWLEDGE_RECORDS * MAX_KNOWLEDGE_BYTES * MAX_KNOWLEDGE_STORES,
            "accepted_general_records": general_records,
            "accepted_general_payload_bytes": general_payload_bytes,
            "durable_records_at_refusal": durable.records,
            "durable_identities_at_refusal": durable.identities,
            "durable_payload_bytes_at_refusal": durable.payload_bytes,
            "refusal": page_refusal,
        },
        "cache": {
            "resident_identity_cap": MAX_KNOWLEDGE_STORES,
            "pinned_snapshot_identities": pinned_identities,
            "refused_257th_pinned_identity": true,
            "records_per_identity_cap": MAX_KNOWLEDGE_RECORDS,
            "max_cached_records_per_identity": max_cache_records,
            "logical_cached_records": cache_records,
            "logical_cached_payload_bytes": cache_payload_bytes,
            "snapshot_handle_inline_bytes": std::mem::size_of::<KnowledgeSnapshot>() * pinned_identities,
            "fact_handle_inline_bytes": std::mem::size_of::<KnowledgeFact>() * cache_records,
            "snapshot_generations_per_identity_cap": 2,
        },
        "queue_and_staging": queue,
        "storage": {
            "database_page_count_at_end": final_storage.0,
            "database_page_count_after_close": final_storage_after_close.0,
            "database_page_count_peak": peak_storage.0,
            "database_page_cap": sql::DB_MAX_PAGES,
            "database_cap_bytes": db_cap_bytes,
            "database_cap_mib": db_cap_bytes as f64 / (1024.0 * 1024.0),
            "wal_bytes_at_end": final_storage.1,
            "wal_bytes_after_close": final_storage_after_close.1,
            "wal_bytes_peak": peak_storage.1,
            "wal_cap_bytes": sql::WAL_MAX_BYTES,
            "wal_frame_cap": sql::WAL_MAX_FRAMES,
        },
        "sqlite_c_heap": {
            "current_bytes_before_close": sqlite_current_before_close,
            "current_bytes_after_close": sqlite_current_after_close,
            "highwater_bytes": sqlite_peak,
            "limit_bytes": 8 * 1024 * 1024,
        },
        "limits": {
            "all_sql_jobs_and_validation_below_250ms": timings_within_limit,
            "sqlite_c_heap_below_8mib": heap_within_limit,
            "database_pages_within_cap": page_within_limit,
            "wal_bytes_within_cap": wal_within_limit,
        },
    });
    std::fs::write(&out, serde_json::to_vec_pretty(&report).unwrap()).unwrap();

    assert!(
        timings_within_limit,
        "SQL writer job or validation exceeded 250 ms: {max_sql_job_ms:.3} ms job, {max_validation_ms:.3} ms validation"
    );
    assert!(
        heap_within_limit,
        "SQLite C heap exceeded 8 MiB: {sqlite_peak}"
    );
    assert!(page_within_limit, "page cap exceeded: {}", peak_storage.0);
    assert!(wal_within_limit, "WAL cap exceeded: {}", peak_storage.1);
}
