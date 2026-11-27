use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use api::identity::{DurableTimestamp, Settlement};
use api::local_state::{
    validate_knowledge_payload, AccountKey, ContentScope, DomainEdit, DomainIdentity, ErrorCode,
    KnowledgeKey, KnowledgeProvenance, KnowledgeRecord, KnowledgeRevocation, KnowledgeStatus,
    LocalStateDomain, StoreVersion, SubmitReceipt, ValidationReason, KNOWLEDGE_SCHEMA_VERSION,
    MAX_KNOWLEDGE_BYTES, MAX_KNOWLEDGE_RECORDS,
};
use client::io::ClientRevision;
use rusqlite::{Connection, OpenFlags};
use script::IsolatedEnv;
use serde_json::json;

use super::{
    run_preferences_maintenance, sql, Frontend, KnowledgeFact, KnowledgeSnapshot,
    LocalStateService, MaintenanceAction, RequestState, StateRoot, MAX_KNOWLEDGE_STORES,
};

const WAIT: Duration = Duration::from_secs(60);
const PROFILE: &str = "local-289";
const CONTENT: &str = "test-content";
const ROUTE_KEY: &str = "upass.grid_route";
const TOMBSTONE_AT: u64 = 1_760_000_000;
const ROUTE_PAYLOAD: [u8; 6] = [5, 4, 5, 15, 16, 12];

fn identity(
    profile: &str,
    account: &str,
    revision: ClientRevision,
    content: &str,
) -> DomainIdentity {
    DomainIdentity::new(
        LocalStateDomain::Knowledge,
        AccountKey::try_new(profile, account).unwrap(),
        ContentScope::try_new(revision, content).unwrap(),
    )
}

fn route_identity() -> DomainIdentity {
    identity(PROFILE, "alice", ClientRevision::R289, CONTENT)
}

fn indexed_identity(index: usize) -> DomainIdentity {
    identity(
        PROFILE,
        &format!("alice{index:03}"),
        ClientRevision::R289,
        CONTENT,
    )
}

fn root_of(scratch: &IsolatedEnv) -> StateRoot {
    StateRoot::new(scratch.home.join(".274bot"))
}

fn knowledge_only(root: StateRoot) -> Arc<LocalStateService> {
    // Exclude unrelated startup imports when asserting the root transaction sequence.
    LocalStateService::open(
        root,
        &[LocalStateDomain::Knowledge],
        Arc::new(api::identity::RequestIdMinter::new()),
    )
}

fn readback(
    service: &LocalStateService,
    identity: &DomainIdentity,
) -> Result<KnowledgeSnapshot, ErrorCode> {
    let ticket = service.start_knowledge_readback(identity)?;
    service.wait_knowledge_readback(identity, ticket, WAIT)
}

fn make_record(
    identity: &DomainIdentity,
    key: &str,
    expected: StoreVersion,
    provenance: KnowledgeProvenance,
    status: KnowledgeStatus,
    bytes: &[u8],
    observed_at: u64,
) -> KnowledgeRecord {
    let payload = validate_knowledge_payload(KNOWLEDGE_SCHEMA_VERSION, bytes).unwrap();
    KnowledgeRecord::try_new(
        identity.account().clone(),
        identity.scope().clone(),
        expected,
        key,
        provenance,
        status,
        payload,
        DurableTimestamp::from_unix_seconds(observed_at),
    )
    .unwrap()
}

fn settle(
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
        other => panic!("knowledge request did not reach a durable terminal: {other:?}"),
    }
}

fn publish(
    service: &LocalStateService,
    record: KnowledgeRecord,
) -> Result<api::local_state::DurableReceipt, ErrorCode> {
    settle(service, service.publish_knowledge(record))
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

fn durable_fact_count(root: &StateRoot) -> usize {
    let count: i64 = inspect(root)
        .query_row("SELECT count(*) FROM knowledge_fact", [], |row| row.get(0))
        .unwrap();
    usize::try_from(count).unwrap()
}

fn table_exists(root: &StateRoot, table: &str) -> bool {
    inspect(root)
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
             )",
            [table],
            |row| row.get::<_, bool>(0),
        )
        .unwrap()
}

fn user_version(root: &StateRoot) -> i32 {
    inspect(root)
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap()
}

fn key(value: &str) -> KnowledgeKey {
    KnowledgeKey::try_new(value).unwrap()
}

fn fact<'a>(snapshot: &'a KnowledgeSnapshot, key: &KnowledgeKey) -> &'a KnowledgeFact {
    snapshot.fact(key).expect("the knowledge fact is present")
}

fn assert_record(
    snapshot: &KnowledgeSnapshot,
    key: &KnowledgeKey,
    status: KnowledgeStatus,
    provenance: KnowledgeProvenance,
    bytes: &[u8],
    observed_at: u64,
    revocation: Option<KnowledgeRevocation>,
) {
    let stored = fact(snapshot, key).record();
    assert_eq!(stored.typed_key(), key);
    assert_eq!(stored.status(), status);
    assert_eq!(stored.provenance(), provenance);
    assert_eq!(stored.schema_version(), KNOWLEDGE_SCHEMA_VERSION);
    assert_eq!(stored.payload(), bytes);
    assert_eq!(
        stored.observed_at(),
        DurableTimestamp::from_unix_seconds(observed_at)
    );
    assert_eq!(stored.revocation(), revocation);
}

#[test]
fn knowledge_round_trips_all_statuses_metadata_and_payloads_after_restart() {
    crate::test_process::isolate_state_test!(
        knowledge_round_trips_all_statuses_metadata_and_payloads_after_restart
    );
    let scratch = IsolatedEnv::enter("knowledge-round-trip");
    let root = root_of(&scratch);
    let identity = route_identity();
    let cases = [
        (
            "fixture.known",
            KnowledgeStatus::Known,
            KnowledgeProvenance::Observed,
            b"known\0\xff".as_slice(),
            TOMBSTONE_AT,
            None,
        ),
        (
            "fixture.partial",
            KnowledgeStatus::Partial,
            KnowledgeProvenance::Observed,
            b"partial\x80bytes".as_slice(),
            TOMBSTONE_AT + 1,
            None,
        ),
        (
            "fixture.unknown",
            KnowledgeStatus::Unknown,
            KnowledgeProvenance::Journal,
            b"unknown-with-payload".as_slice(),
            TOMBSTONE_AT + 2,
            None,
        ),
    ];

    let service = LocalStateService::open_process(root.clone());
    drop(readback(&service, &identity).unwrap());
    for (name, status, provenance, bytes, observed_at, revocation) in cases {
        let record = make_record(
            &identity,
            name,
            StoreVersion::INITIAL,
            provenance,
            status,
            bytes,
            observed_at,
        );
        let edit = DomainEdit::Knowledge(record.clone());
        assert_eq!(edit.expected(), Some(StoreVersion::INITIAL));
        assert!(edit.applies_to(StoreVersion::INITIAL));
        assert!(!edit.applies_to(StoreVersion::from_durable(1)));
        publish(&service, record).unwrap();
        assert_eq!(revocation, None);
    }
    let before_restart = readback(&service, &identity).unwrap();
    assert_eq!(before_restart.identity(), &identity);
    assert_eq!(before_restart.records().len(), cases.len());
    for (name, status, provenance, bytes, observed_at, revocation) in cases {
        assert_record(
            &before_restart,
            &key(name),
            status,
            provenance,
            bytes,
            observed_at,
            revocation,
        );
        let stored = fact(&before_restart, &key(name));
        assert!(stored.version() > StoreVersion::INITIAL);
        assert_eq!(stored.version().get(), stored.transaction_sequence());
    }
    drop(before_restart);
    drop(service);

    let reopened = LocalStateService::open_process(root);
    let after_restart = readback(&reopened, &identity).unwrap();
    assert_eq!(after_restart.records().len(), cases.len());
    for (name, status, provenance, bytes, observed_at, revocation) in cases {
        assert_record(
            &after_restart,
            &key(name),
            status,
            provenance,
            bytes,
            observed_at,
            revocation,
        );
    }
}

#[test]
fn absent_knowledge_read_does_not_write_facts_or_advance_sequence() {
    crate::test_process::isolate_state_test!(
        absent_knowledge_read_does_not_write_facts_or_advance_sequence
    );
    let scratch = IsolatedEnv::enter("knowledge-absent-read");
    let root = root_of(&scratch);
    let service = knowledge_only(root.clone());
    let first_identity = route_identity();
    let first = readback(&service, &first_identity).unwrap();
    let absent_key = key(ROUTE_KEY);
    assert!(first.records().is_empty());
    assert_eq!(first.status(&absent_key), KnowledgeStatus::Unknown);
    assert_eq!(first.version(&absent_key), StoreVersion::INITIAL);
    assert!(first.fact(&absent_key).is_none());
    drop(first);

    let sequence_before = database_sequence(&root);
    let facts_before = durable_fact_count(&root);
    assert_eq!(facts_before, 0);
    let second_identity = identity(PROFILE, "bob", ClientRevision::R289, CONTENT);
    let second = readback(&service, &second_identity).unwrap();
    assert!(second.records().is_empty());
    assert_eq!(second.status(&absent_key), KnowledgeStatus::Unknown);
    assert_eq!(second.version(&absent_key), StoreVersion::INITIAL);
    drop(second);

    assert_eq!(database_sequence(&root), sequence_before);
    assert_eq!(durable_fact_count(&root), facts_before);
}

#[test]
fn knowledge_is_isolated_by_full_account_revision_and_content_identity() {
    crate::test_process::isolate_state_test!(
        knowledge_is_isolated_by_full_account_revision_and_content_identity
    );
    let scratch = IsolatedEnv::enter("knowledge-identity-isolation");
    let service = LocalStateService::open_process(root_of(&scratch));
    let identities = [
        identity(PROFILE, "alice", ClientRevision::R289, "content-a"),
        identity(PROFILE, "bob", ClientRevision::R289, "content-a"),
        identity("other-profile", "alice", ClientRevision::R289, "content-a"),
        identity(PROFILE, "alice", ClientRevision::R274, "content-a"),
        identity(PROFILE, "alice", ClientRevision::R289, "content-b"),
    ];
    let shared_key = key("fixture.same");
    let mut snapshots = Vec::new();

    for (index, identity) in identities.iter().enumerate() {
        drop(readback(&service, identity).unwrap());
        let bytes = [index as u8, 0, 0xff];
        publish(
            &service,
            make_record(
                identity,
                shared_key.as_str(),
                StoreVersion::INITIAL,
                KnowledgeProvenance::Observed,
                KnowledgeStatus::Known,
                &bytes,
                TOMBSTONE_AT + index as u64,
            ),
        )
        .unwrap();
        snapshots.push(readback(&service, identity).unwrap());
    }

    for (index, (identity, snapshot)) in identities.iter().zip(&snapshots).enumerate() {
        assert_eq!(snapshot.identity(), identity);
        assert_eq!(snapshot.records().len(), 1);
        assert_record(
            snapshot,
            &shared_key,
            KnowledgeStatus::Known,
            KnowledgeProvenance::Observed,
            &[index as u8, 0, 0xff],
            TOMBSTONE_AT + index as u64,
            None,
        );
    }
}

#[test]
fn knowledge_writes_use_per_fact_compare_and_swap() {
    crate::test_process::isolate_state_test!(knowledge_writes_use_per_fact_compare_and_swap);
    let scratch = IsolatedEnv::enter("knowledge-cas");
    let root = root_of(&scratch);
    let service = LocalStateService::open_process(root.clone());
    let identity = route_identity();
    let key = key("fixture.cas");
    drop(readback(&service, &identity).unwrap());

    let winner = make_record(
        &identity,
        key.as_str(),
        StoreVersion::INITIAL,
        KnowledgeProvenance::Observed,
        KnowledgeStatus::Known,
        b"winner",
        TOMBSTONE_AT,
    );
    let stale = make_record(
        &identity,
        key.as_str(),
        StoreVersion::INITIAL,
        KnowledgeProvenance::Observed,
        KnowledgeStatus::Partial,
        b"stale",
        TOMBSTONE_AT,
    );
    let winner_receipt = publish(&service, winner).unwrap();
    let winner_version = winner_receipt.version();
    let sequence_before_stale = database_sequence(&root);
    let conflict = publish(&service, stale).unwrap_err();
    assert_eq!(
        conflict,
        ErrorCode::Conflict {
            expected: StoreVersion::INITIAL,
            found: winner_version,
        }
    );
    assert_eq!(database_sequence(&root), sequence_before_stale);

    let current = service.knowledge_snapshot(&identity).unwrap();
    assert_eq!(current.version(&key), winner_version);
    assert_record(
        &current,
        &key,
        KnowledgeStatus::Known,
        KnowledgeProvenance::Observed,
        b"winner",
        TOMBSTONE_AT,
        None,
    );
}

#[test]
fn refusal_tombstone_blocks_same_second_stale_retry_in_a_fresh_process() {
    crate::test_process::isolate_state_test!(
        refusal_tombstone_blocks_same_second_stale_retry_in_a_fresh_process
    );
    let scratch = IsolatedEnv::enter("knowledge-same-second-refusal");
    let root = root_of(&scratch);
    let identity = route_identity();
    let key = key(ROUTE_KEY);
    let service = LocalStateService::open_process(root.clone());
    drop(readback(&service, &identity).unwrap());

    let stale_positive = make_record(
        &identity,
        ROUTE_KEY,
        StoreVersion::INITIAL,
        KnowledgeProvenance::Observed,
        KnowledgeStatus::Known,
        &ROUTE_PAYLOAD,
        TOMBSTONE_AT,
    );
    let known_receipt = publish(&service, stale_positive.clone()).unwrap();
    let known_version = known_receipt.version();
    let known_snapshot = service.knowledge_snapshot(&identity).unwrap();
    assert_record(
        &known_snapshot,
        &key,
        KnowledgeStatus::Known,
        KnowledgeProvenance::Observed,
        &ROUTE_PAYLOAD,
        TOMBSTONE_AT,
        None,
    );

    let tombstone_receipt = settle(
        &service,
        service.invalidate_knowledge(
            &identity,
            key.clone(),
            known_version,
            DurableTimestamp::from_unix_seconds(TOMBSTONE_AT),
            KnowledgeRevocation::ObservationRefused,
        ),
    )
    .unwrap();
    let tombstone_version = tombstone_receipt.version();
    let tombstone_snapshot = service.knowledge_snapshot(&identity).unwrap();
    assert_record(
        &tombstone_snapshot,
        &key,
        KnowledgeStatus::Unknown,
        KnowledgeProvenance::Observed,
        &[],
        TOMBSTONE_AT,
        Some(KnowledgeRevocation::ObservationRefused),
    );
    assert_eq!(fact(&tombstone_snapshot, &key).version(), tombstone_version);

    assert_record(
        &known_snapshot,
        &key,
        KnowledgeStatus::Known,
        KnowledgeProvenance::Observed,
        &ROUTE_PAYLOAD,
        TOMBSTONE_AT,
        None,
    );
    drop(known_snapshot);
    let sequence_before_retry = database_sequence(&root);
    let stale_result = publish(&service, stale_positive).unwrap_err();
    assert_eq!(
        stale_result,
        ErrorCode::Conflict {
            expected: StoreVersion::INITIAL,
            found: tombstone_version,
        }
    );
    assert_eq!(database_sequence(&root), sequence_before_retry);
    assert_record(
        &tombstone_snapshot,
        &key,
        KnowledgeStatus::Unknown,
        KnowledgeProvenance::Observed,
        &[],
        TOMBSTONE_AT,
        Some(KnowledgeRevocation::ObservationRefused),
    );
    drop(tombstone_snapshot);
    drop(service);

    crate::test_process::run(
        "local_state::knowledge_tests::knowledge_tombstone_restart_child",
        &[("KNOWLEDGE_CHILD_ROOT", root.path().to_str().unwrap())],
    );
}

#[test]
#[ignore = "fresh-process readback child for the same-second tombstone test"]
fn knowledge_tombstone_restart_child() {
    let root = StateRoot::new(PathBuf::from(
        std::env::var_os("KNOWLEDGE_CHILD_ROOT").expect("KNOWLEDGE_CHILD_ROOT is set"),
    ));
    let identity = route_identity();
    let key = key(ROUTE_KEY);
    let service = LocalStateService::open_process(root);
    let snapshot = readback(&service, &identity).unwrap();
    assert_record(
        &snapshot,
        &key,
        KnowledgeStatus::Unknown,
        KnowledgeProvenance::Observed,
        &[],
        TOMBSTONE_AT,
        Some(KnowledgeRevocation::ObservationRefused),
    );
    let stored = fact(&snapshot, &key);
    assert_eq!(stored.version().get(), stored.transaction_sequence());
}

#[test]
fn knowledge_256_pinned_snapshots_refuse_another_store_then_allow_eviction() {
    crate::test_process::isolate_state_test!(
        knowledge_256_pinned_snapshots_refuse_another_store_then_allow_eviction
    );
    let scratch = IsolatedEnv::enter("knowledge-pinned-capacity");
    let service = LocalStateService::open_process(root_of(&scratch));
    let saved_identity = indexed_identity(0);
    let saved_key = key("fixture.saved");
    drop(readback(&service, &saved_identity).unwrap());
    publish(
        &service,
        make_record(
            &saved_identity,
            saved_key.as_str(),
            StoreVersion::INITIAL,
            KnowledgeProvenance::Observed,
            KnowledgeStatus::Known,
            b"durable-before-eviction",
            TOMBSTONE_AT,
        ),
    )
    .unwrap();

    let mut pinned = Vec::with_capacity(MAX_KNOWLEDGE_STORES);
    pinned.push(service.knowledge_snapshot(&saved_identity).unwrap());
    for index in 1..MAX_KNOWLEDGE_STORES {
        pinned.push(readback(&service, &indexed_identity(index)).unwrap());
    }

    let extra_identity = indexed_identity(MAX_KNOWLEDGE_STORES);
    let error = readback(&service, &extra_identity).unwrap_err();
    assert!(matches!(
        error,
        ErrorCode::Busy {
            domain: LocalStateDomain::Knowledge
        }
    ));

    drop(pinned);
    let extra = readback(&service, &extra_identity).unwrap();
    assert!(extra.records().is_empty());
    assert_eq!(extra.status(&saved_key), KnowledgeStatus::Unknown);
    assert!(matches!(
        service.knowledge_snapshot(&saved_identity),
        Err(ErrorCode::NotReady {
            domain: LocalStateDomain::Knowledge
        })
    ));

    let rehydrated = readback(&service, &saved_identity).unwrap();
    assert_record(
        &rehydrated,
        &saved_key,
        KnowledgeStatus::Known,
        KnowledgeProvenance::Observed,
        b"durable-before-eviction",
        TOMBSTONE_AT,
        None,
    );
}

#[test]
fn knowledge_caps_payloads_and_records_without_losing_prior_facts() {
    crate::test_process::isolate_state_test!(
        knowledge_caps_payloads_and_records_without_losing_prior_facts
    );
    let scratch = IsolatedEnv::enter("knowledge-record-cap");
    let root = root_of(&scratch);
    let service = knowledge_only(root.clone());
    let identity = route_identity();
    let mut snapshot = readback(&service, &identity).unwrap();
    let oversized = vec![0x7f; MAX_KNOWLEDGE_BYTES + 1];
    assert_eq!(
        validate_knowledge_payload(KNOWLEDGE_SCHEMA_VERSION, &oversized).unwrap_err(),
        ErrorCode::Validation {
            domain: LocalStateDomain::Knowledge,
            reason: ValidationReason::TooLong,
        }
    );
    assert_eq!(database_sequence(&root), 0);
    assert_eq!(durable_fact_count(&root), 0);

    let mut expected = BTreeMap::new();
    for index in 0..MAX_KNOWLEDGE_RECORDS {
        let name = format!("bulk.fact_{index:02}");
        let bytes = if index == 0 {
            vec![0xa5; MAX_KNOWLEDGE_BYTES]
        } else {
            format!("fixture-payload-{index:02}").into_bytes()
        };
        let expected_version = snapshot.version(&key(&name));
        let receipt = publish(
            &service,
            make_record(
                &identity,
                &name,
                expected_version,
                KnowledgeProvenance::Observed,
                KnowledgeStatus::Known,
                &bytes,
                TOMBSTONE_AT + index as u64,
            ),
        )
        .unwrap();
        assert!(receipt.version() > StoreVersion::INITIAL);
        expected.insert(name, bytes);
        snapshot = service.knowledge_snapshot(&identity).unwrap();
    }
    assert_eq!(snapshot.records().len(), MAX_KNOWLEDGE_RECORDS);
    let sequence_before_refusal = database_sequence(&root);
    let rows_before_refusal = durable_fact_count(&root);
    let too_many = publish(
        &service,
        make_record(
            &identity,
            "bulk.fact_64",
            snapshot.version(&key("bulk.fact_64")),
            KnowledgeProvenance::Observed,
            KnowledgeStatus::Known,
            b"must-not-replace",
            TOMBSTONE_AT + MAX_KNOWLEDGE_RECORDS as u64,
        ),
    )
    .unwrap_err();
    assert!(matches!(
        too_many,
        ErrorCode::Busy {
            domain: LocalStateDomain::Knowledge
        } | ErrorCode::Validation {
            domain: LocalStateDomain::Knowledge,
            reason: ValidationReason::TooManyRows,
        }
    ));
    assert_eq!(database_sequence(&root), sequence_before_refusal);
    assert_eq!(durable_fact_count(&root), rows_before_refusal);
    assert_eq!(rows_before_refusal, MAX_KNOWLEDGE_RECORDS);

    let current = service.knowledge_snapshot(&identity).unwrap();
    assert_eq!(current.records().len(), MAX_KNOWLEDGE_RECORDS);
    for (name, bytes) in &expected {
        let current_key = key(name);
        assert_eq!(fact(&current, &current_key).record().payload(), bytes);
    }
}

#[test]
fn knowledge_refuses_a_third_snapshot_generation_until_the_old_lease_drops() {
    crate::test_process::isolate_state_test!(
        knowledge_refuses_a_third_snapshot_generation_until_the_old_lease_drops
    );
    let scratch = IsolatedEnv::enter("knowledge-generation-cap");
    let root = root_of(&scratch);
    let service = LocalStateService::open_process(root.clone());
    let identity = route_identity();
    let key = key("fixture.generation");
    drop(readback(&service, &identity).unwrap());

    publish(
        &service,
        make_record(
            &identity,
            key.as_str(),
            StoreVersion::INITIAL,
            KnowledgeProvenance::Observed,
            KnowledgeStatus::Known,
            b"generation-one",
            TOMBSTONE_AT,
        ),
    )
    .unwrap();
    let first = service.knowledge_snapshot(&identity).unwrap();
    let second_record = make_record(
        &identity,
        key.as_str(),
        first.version(&key),
        KnowledgeProvenance::Observed,
        KnowledgeStatus::Known,
        b"generation-two",
        TOMBSTONE_AT,
    );
    publish(&service, second_record).unwrap();
    let second = service.knowledge_snapshot(&identity).unwrap();
    let third_record = make_record(
        &identity,
        key.as_str(),
        second.version(&key),
        KnowledgeProvenance::Observed,
        KnowledgeStatus::Known,
        b"generation-three",
        TOMBSTONE_AT,
    );

    let sequence_before_refusal = database_sequence(&root);
    let refusal = publish(&service, third_record.clone()).unwrap_err();
    assert!(matches!(
        refusal,
        ErrorCode::Busy {
            domain: LocalStateDomain::Knowledge
        }
    ));
    assert_eq!(database_sequence(&root), sequence_before_refusal);
    assert_eq!(fact(&first, &key).record().payload(), b"generation-one");
    assert_eq!(fact(&second, &key).record().payload(), b"generation-two");

    drop(first);
    publish(&service, third_record).unwrap();
    let third = service.knowledge_snapshot(&identity).unwrap();
    assert_eq!(fact(&second, &key).record().payload(), b"generation-two");
    assert_eq!(fact(&third, &key).record().payload(), b"generation-three");
}

#[test]
fn synthetic_runner_knowledge_json_is_ignored() {
    crate::test_process::isolate_state_test!(synthetic_runner_knowledge_json_is_ignored);
    let scratch = IsolatedEnv::enter("knowledge-runner-json-ignored");
    let root = root_of(&scratch);
    let identity = route_identity();
    let source = root
        .state_dir()
        .join("knowledge")
        .join(PROFILE)
        .join("alice.json");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    let runner_bytes = serde_json::to_vec(&json!({
        "schema_version": 1,
        "profile": PROFILE,
        "account": "alice",
        "revision": 289,
        "content": CONTENT,
        "entries": [{
            "key": ROUTE_KEY,
            "provenance": "observed",
            "status": "known",
            "schema": 1,
            "observed_at_unix": TOMBSTONE_AT,
            "payload": ROUTE_PAYLOAD,
        }],
    }))
    .unwrap();
    std::fs::write(&source, &runner_bytes).unwrap();

    let service = LocalStateService::open_process(root.clone());
    let snapshot = readback(&service, &identity).unwrap();
    let route = key(ROUTE_KEY);
    assert!(snapshot.records().is_empty());
    assert!(snapshot.fact(&route).is_none());
    assert_eq!(snapshot.status(&route), KnowledgeStatus::Unknown);
    assert_eq!(snapshot.version(&route), StoreVersion::INITIAL);
    assert_eq!(durable_fact_count(&root), 0);
    assert_eq!(std::fs::read(&source).unwrap(), runner_bytes);
}

#[test]
fn schema_five_migrates_to_knowledge_schema_six() {
    crate::test_process::isolate_state_test!(schema_five_migrates_to_knowledge_schema_six);
    let scratch = IsolatedEnv::enter("knowledge-schema-five-migration");
    let root = root_of(&scratch);
    let identity = route_identity();
    let first = LocalStateService::open_process(root.clone());
    let snapshot = readback(&first, &identity).unwrap();
    assert!(snapshot.records().is_empty());
    assert_eq!(user_version(&root), super::sql::USER_VERSION);
    assert!(table_exists(&root, "knowledge_fact"));
    drop(snapshot);
    drop(first);

    inspect(&root)
        .execute_batch(
            "DROP TABLE knowledge_fact; DROP TABLE profile_setting; DROP TABLE profile_row;
             DROP TABLE vault_pending; DROP TABLE vault_binding; PRAGMA user_version = 5;",
        )
        .unwrap();
    assert_eq!(user_version(&root), 5);
    assert!(!table_exists(&root, "knowledge_fact"));

    let reopened = LocalStateService::open_process(root.clone());
    let migrated = readback(&reopened, &identity).unwrap();
    assert!(migrated.records().is_empty());
    assert_eq!(migrated.status(&key(ROUTE_KEY)), KnowledgeStatus::Unknown);
    assert_eq!(user_version(&root), super::sql::USER_VERSION);
    assert!(table_exists(&root, "knowledge_fact"));
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExpectedFact {
    key: KnowledgeKey,
    version: StoreVersion,
    transaction_sequence: u64,
    schema_version: u32,
    provenance: KnowledgeProvenance,
    observed_at: DurableTimestamp,
    status: KnowledgeStatus,
    revocation: Option<KnowledgeRevocation>,
    payload: Vec<u8>,
}

fn expected_fact(snapshot: &KnowledgeSnapshot, key: &KnowledgeKey) -> ExpectedFact {
    let fact = fact(snapshot, key);
    let record = fact.record();
    ExpectedFact {
        key: key.clone(),
        version: fact.version(),
        transaction_sequence: fact.transaction_sequence(),
        schema_version: record.schema_version(),
        provenance: record.provenance(),
        observed_at: record.observed_at(),
        status: record.status(),
        revocation: record.revocation(),
        payload: record.payload().to_vec(),
    }
}

fn assert_expected_fact(snapshot: &KnowledgeSnapshot, expected: &ExpectedFact) {
    let fact = fact(snapshot, &expected.key);
    let record = fact.record();
    assert_eq!(fact.version(), expected.version);
    assert_eq!(fact.transaction_sequence(), expected.transaction_sequence);
    assert_eq!(fact.version().get(), fact.transaction_sequence());
    assert_eq!(record.typed_key(), &expected.key);
    assert_eq!(record.schema_version(), expected.schema_version);
    assert_eq!(record.provenance(), expected.provenance);
    assert_eq!(record.observed_at(), expected.observed_at);
    assert_eq!(record.status(), expected.status);
    assert_eq!(record.revocation(), expected.revocation);
    assert_eq!(record.payload(), expected.payload);
}

#[test]
fn preferences_defaults_recovery_preserves_knowledge_facts_versions_and_tombstones() {
    crate::test_process::isolate_state_test!(
        preferences_defaults_recovery_preserves_knowledge_facts_versions_and_tombstones
    );
    let scratch = IsolatedEnv::enter("knowledge-defaults-recovery");
    let root = root_of(&scratch);
    let identity = route_identity();
    let known_key = key("recovery.known");
    let partial_key = key("recovery.partial");
    let refused_key = key("recovery.refused");
    let expected;
    let sequence_before;

    {
        let service = LocalStateService::open_process(root.clone());
        drop(readback(&service, &identity).unwrap());
        publish(
            &service,
            make_record(
                &identity,
                known_key.as_str(),
                StoreVersion::INITIAL,
                KnowledgeProvenance::Observed,
                KnowledgeStatus::Known,
                b"recovery-known-payload",
                TOMBSTONE_AT,
            ),
        )
        .unwrap();
        publish(
            &service,
            make_record(
                &identity,
                partial_key.as_str(),
                StoreVersion::INITIAL,
                KnowledgeProvenance::Journal,
                KnowledgeStatus::Partial,
                b"recovery-partial-payload",
                TOMBSTONE_AT + 1,
            ),
        )
        .unwrap();
        let refused_source = make_record(
            &identity,
            refused_key.as_str(),
            StoreVersion::INITIAL,
            KnowledgeProvenance::Observed,
            KnowledgeStatus::Known,
            b"source-before-refusal",
            TOMBSTONE_AT + 2,
        );
        let refused_version = publish(&service, refused_source).unwrap().version();
        settle(
            &service,
            service.invalidate_knowledge(
                &identity,
                refused_key.clone(),
                refused_version,
                DurableTimestamp::from_unix_seconds(TOMBSTONE_AT + 2),
                KnowledgeRevocation::ContentChanged,
            ),
        )
        .unwrap();

        let snapshot = service.knowledge_snapshot(&identity).unwrap();
        expected = [
            expected_fact(&snapshot, &known_key),
            expected_fact(&snapshot, &partial_key),
            expected_fact(&snapshot, &refused_key),
        ];
        assert_eq!(snapshot.records().len(), expected.len());
        assert_eq!(
            snapshot.fact(&refused_key).unwrap().record().revocation(),
            Some(KnowledgeRevocation::ContentChanged)
        );
        sequence_before = database_sequence(&root);
        assert_eq!(
            sequence_before,
            expected
                .iter()
                .map(|fact| fact.transaction_sequence)
                .max()
                .unwrap()
        );
        drop(snapshot);
    }

    run_preferences_maintenance(&root, Frontend::Panel, &MaintenanceAction::RecoverDefaults)
        .unwrap();

    let recovered = LocalStateService::open_process(root.clone());
    let snapshot = readback(&recovered, &identity).unwrap();
    assert_eq!(snapshot.records().len(), expected.len());
    for expected in &expected {
        assert_expected_fact(&snapshot, expected);
    }
    assert_eq!(database_sequence(&root), sequence_before);
    assert_eq!(durable_fact_count(&root), expected.len());
}
