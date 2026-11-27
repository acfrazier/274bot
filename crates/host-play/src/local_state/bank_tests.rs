//! S5a: the bank hints in the state database, with real SQLite and real
//! files. The import of the legacy `bank-hints/<profile>/<account>.json`,
//! the one stored form, and the saves through the one commit path.

use super::*;
use crate::local_state::BankHintBinding;
use std::path::{Path, PathBuf};
use std::sync::Barrier;

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Value};

const OTHER_CONTENT: &str = "other-content";

fn root_of(scratch: &IsolatedEnv) -> StateRoot {
    StateRoot::new(scratch.home.join(".274bot"))
}

fn bank_service(scratch: &IsolatedEnv) -> Arc<LocalStateService> {
    service(scratch, &[LocalStateDomain::BankHints])
}

/// A connection of the test's own, for inspection.
fn inspect(root: &StateRoot) -> Connection {
    let conn = Connection::open_with_flags(
        root.database_file(),
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    conn.busy_timeout(Duration::from_secs(5)).unwrap();
    conn
}

/// `<file>.migrated`, where the first import keeps the original.
fn migrated(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap().to_os_string();
    name.push(".migrated");
    path.with_file_name(name)
}

/// What the ledger records for one account's source: kind and evidence.
fn ledger(root: &StateRoot, account: &str) -> Option<(String, String)> {
    inspect(root)
        .query_row(
            "SELECT kind, evidence FROM import_ledger
             WHERE domain = 'bank-hints' AND source = ?1",
            params![format!("{PROFILE}/{account}")],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .unwrap()
}

/// Every stored hint: account, revision, content, observed time, rows,
/// omitted fields.
type StoredHint = (String, i64, String, i64, Vec<(i32, i32)>, i64);

fn stored(root: &StateRoot) -> Vec<StoredHint> {
    let conn = inspect(root);
    let mut statement = conn
        .prepare(
            "SELECT account, revision, content, observed_at, rows, omitted FROM bank_hint
             ORDER BY account, revision, content",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            let raw: Vec<u8> = row.get(4)?;
            let rows = raw
                .as_chunks::<8>()
                .0
                .iter()
                .map(|row| {
                    (
                        i32::from_be_bytes([row[0], row[1], row[2], row[3]]),
                        i32::from_be_bytes([row[4], row[5], row[6], row[7]]),
                    )
                })
                .collect();
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                rows,
                row.get(5)?,
            ))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// A schema-1 hint document of the 0.2.9 format, with its scope or, for a
/// 0.2.8 file, none, and any extra members.
fn legacy_document(
    account: &str,
    scope: Option<(i32, &str)>,
    observed_at: u64,
    rows: &[(i32, i32)],
    extra: &[(&str, Value)],
) -> Vec<u8> {
    let mut document = json!({
        "schema_version": HINT_SCHEMA_VERSION,
        "profile": PROFILE,
        "account": account,
        "observed_at_unix": observed_at,
        "rows": rows,
    });
    let fields = document.as_object_mut().unwrap();
    if let Some((revision, content)) = scope {
        fields.insert("revision".into(), json!(revision));
        fields.insert("content".into(), json!(content));
    }
    for (name, value) in extra {
        fields.insert((*name).into(), value.clone());
    }
    serde_json::to_vec(&document).unwrap()
}

fn refusal_of(reason: FailureReason) -> ErrorCode {
    ErrorCode::Failed {
        domain: LocalStateDomain::BankHints,
        reason,
    }
}

/// A scoped schema-1 hint (the 0.2.9 format) and a hint without a scope (the
/// 0.2.8 format) both import, once, with their rows and time; a restart
/// reads the same payload and version, and the original is kept as evidence.
#[test]
fn a_scoped_schema_1_hint_imports_once_and_keeps_its_payload_across_a_restart() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-import-scoped");
    let root = root_of(&scratch);
    let source = root.bank_hint_file(PROFILE, "alice");
    let rows = [(379, 12), (995, 3400)];
    let bytes = legacy_document("alice", Some((289, CONTENT)), 1_759_700_123, &rows, &[]);
    write_private(&source, &bytes);
    let alice = bank_identity("alice");

    let first = {
        let service = bank_service(&scratch);
        let lease = service
            .readback_lease(&alice, WAIT)
            .unwrap()
            .expect("the import made a stored hint");
        assert_eq!(lease.snapshot().as_bank_hints().unwrap(), rows);
        assert_eq!(lease.at().unix_seconds(), 1_759_700_123);
        lease.version()
    };

    assert!(!source.exists(), "the source moved to its evidence name");
    assert_eq!(std::fs::read(migrated(&source)).unwrap(), bytes);
    assert_eq!(
        ledger(&root, "alice"),
        Some(("imported".into(), "completed".into()))
    );
    assert_eq!(
        stored(&root),
        vec![(
            "alice".into(),
            289,
            CONTENT.into(),
            1_759_700_123,
            rows.to_vec(),
            0
        )]
    );

    let service = bank_service(&scratch);
    let lease = service.readback_lease(&alice, WAIT).unwrap().unwrap();
    assert_eq!(lease.snapshot().as_bank_hints().unwrap(), rows);
    assert_eq!(lease.at().unix_seconds(), 1_759_700_123);
    assert_eq!(
        lease.version(),
        first,
        "the stored version survives a restart"
    );
}

/// A schema-1 hint without a scope takes the scope that asks, once: a second
/// scope of the account starts empty and is never given a copy.
#[test]
fn a_schema_1_hint_without_scope_imports_under_the_requesting_scope_only() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-import-unscoped");
    let root = root_of(&scratch);
    let source = root.bank_hint_file(PROFILE, "alice");
    let bytes = legacy_document("alice", None, 3, &[(995, 3)], &[]);
    write_private(&source, &bytes);
    let under_a = scoped(LocalStateDomain::BankHints, "alice", "content-a");
    let under_b = scoped(LocalStateDomain::BankHints, "alice", OTHER_CONTENT);

    {
        let service = bank_service(&scratch);
        let lease = service.readback_lease(&under_a, WAIT).unwrap().unwrap();
        assert_eq!(lease.snapshot().as_bank_hints().unwrap(), &[(995, 3)]);
        assert_eq!(
            service.readback_lease(&under_b, WAIT).unwrap(),
            None,
            "the second scope does not get a copy"
        );
    }
    assert_eq!(std::fs::read(migrated(&source)).unwrap(), bytes);
    assert_eq!(stored(&root).len(), 1);
    assert_eq!(stored(&root)[0].2, "content-a");

    let service = bank_service(&scratch);
    assert_eq!(
        service
            .readback_lease(&under_a, WAIT)
            .unwrap()
            .unwrap()
            .snapshot()
            .as_bank_hints()
            .unwrap(),
        &[(995, 3)]
    );
    assert_eq!(service.readback_lease(&under_b, WAIT).unwrap(), None);
}

/// Two scopes of one account ask for the same unscoped source at the same
/// moment, through two services on the root: one takes it, once.
#[test]
fn two_scopes_racing_for_one_unscoped_source_never_both_claim_it() {
    let _state_lease = crate::test_process::state();
    for round in 0..4 {
        let scratch = IsolatedEnv::enter(&format!("bank-import-race-{round}"));
        let root = root_of(&scratch);
        write_private(
            &root.bank_hint_file(PROFILE, "alice"),
            &legacy_document("alice", None, 9, &[(995, 9)], &[]),
        );
        let under_a = scoped(LocalStateDomain::BankHints, "alice", "content-a");
        let under_b = scoped(LocalStateDomain::BankHints, "alice", OTHER_CONTENT);
        let first = bank_service(&scratch);
        let second = bank_service(&scratch);
        let barrier = Arc::new(Barrier::new(2));
        let racers: Vec<_> = [(first, under_a), (second, under_b)]
            .into_iter()
            .map(|(service, identity)| {
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    service.readback_lease(&identity, WAIT).unwrap().is_some()
                })
            })
            .collect();
        let claimed: Vec<bool> = racers.into_iter().map(|r| r.join().unwrap()).collect();
        assert_eq!(
            claimed.iter().filter(|claimed| **claimed).count(),
            1,
            "round {round}: exactly one scope takes the source: {claimed:?}"
        );
        assert_eq!(stored(&root).len(), 1, "round {round}");
        assert_eq!(
            ledger(&root, "alice"),
            Some(("imported".into(), "completed".into()))
        );
    }
}

/// A source that records another scope is not consumed by a read for this
/// one, which starts empty; the scope it records still gets it.
#[test]
fn a_hint_recorded_for_another_scope_waits_for_that_scope() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-import-other-scope");
    let root = root_of(&scratch);
    let source = root.bank_hint_file(PROFILE, "alice");
    let bytes = legacy_document("alice", Some((289, "content-a")), 7, &[(995, 7)], &[]);
    write_private(&source, &bytes);
    let under_a = scoped(LocalStateDomain::BankHints, "alice", "content-a");
    let under_b = scoped(LocalStateDomain::BankHints, "alice", "content-b");

    let service = bank_service(&scratch);
    assert_eq!(service.readback_lease(&under_b, WAIT).unwrap(), None);
    assert_eq!(std::fs::read(&source).unwrap(), bytes, "not consumed");
    assert_eq!(ledger(&root, "alice"), None, "no disposition is recorded");
    assert!(stored(&root).is_empty());

    let lease = service.readback_lease(&under_a, WAIT).unwrap().unwrap();
    assert_eq!(lease.snapshot().as_bank_hints().unwrap(), &[(995, 7)]);
    assert!(!source.exists());
    assert_eq!(service.readback_lease(&under_b, WAIT).unwrap(), None);
}

/// A source recorded for another scope never blocks this scope's own saves:
/// the new scope saves before any import of that source, across a restart,
/// and the recorded scope still gets its source afterwards.
#[test]
fn a_hint_recorded_for_another_scope_does_not_block_this_scopes_saves() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-other-scope-save");
    let root = root_of(&scratch);
    let source = root.bank_hint_file(PROFILE, "alice");
    let bytes = legacy_document("alice", Some((289, "content-a")), 7, &[(995, 7)], &[]);
    write_private(&source, &bytes);
    let under_a = scoped(LocalStateDomain::BankHints, "alice", "content-a");
    let under_b = scoped(LocalStateDomain::BankHints, "alice", "content-b");

    let service = bank_service(&scratch);
    assert_eq!(service.readback_lease(&under_b, WAIT).unwrap(), None);
    let saved = request(service.submit(hint_edit(&under_b, None, vec![(995, 3)], 9)));
    completed(&service, saved);
    assert_eq!(std::fs::read(&source).unwrap(), bytes, "not consumed");
    drop(service);

    let service = bank_service(&scratch);
    let lease = service.readback_lease(&under_b, WAIT).unwrap().unwrap();
    assert_eq!(lease.snapshot().as_bank_hints().unwrap(), &[(995, 3)]);
    assert_eq!(std::fs::read(&source).unwrap(), bytes, "still not consumed");
    let lease = service.readback_lease(&under_a, WAIT).unwrap().unwrap();
    assert_eq!(lease.snapshot().as_bank_hints().unwrap(), &[(995, 7)]);
    assert!(!source.exists(), "the recorded scope took its source");
    let lease = service.readback_lease(&under_b, WAIT).unwrap().unwrap();
    assert_eq!(lease.snapshot().as_bank_hints().unwrap(), &[(995, 3)]);
}

/// After the first import, a changed or a new file at the old path is never
/// read: the database is the only authority, and a restart keeps what it
/// holds. A source that was absent stays absent.
#[test]
fn a_legacy_file_after_the_first_import_is_never_read() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-import-closed");
    let root = root_of(&scratch);
    let alice = bank_identity("alice");
    let bob = bank_identity("bob");
    write_private(
        &root.bank_hint_file(PROFILE, "alice"),
        &legacy_document("alice", Some((289, CONTENT)), 1, &[(995, 1)], &[]),
    );
    {
        let service = bank_service(&scratch);
        assert!(service.readback_lease(&alice, WAIT).unwrap().is_some());
        assert_eq!(service.readback_lease(&bob, WAIT).unwrap(), None);
    }
    assert_eq!(
        ledger(&root, "bob"),
        Some(("absent".into(), "none".into())),
        "an absent source is a recorded disposition"
    );

    let changed = legacy_document("alice", Some((289, CONTENT)), 2, &[(995, 2)], &[]);
    write_private(&root.bank_hint_file(PROFILE, "alice"), &changed);
    let appeared = legacy_document("bob", Some((289, CONTENT)), 3, &[(995, 3)], &[]);
    write_private(&root.bank_hint_file(PROFILE, "bob"), &appeared);

    let service = bank_service(&scratch);
    let lease = service.readback_lease(&alice, WAIT).unwrap().unwrap();
    assert_eq!(lease.snapshot().as_bank_hints().unwrap(), &[(995, 1)]);
    assert_eq!(service.readback_lease(&bob, WAIT).unwrap(), None);
    assert_eq!(
        std::fs::read(root.bank_hint_file(PROFILE, "alice")).unwrap(),
        changed,
        "the later file is left where it is"
    );
}

/// What the strict reader refuses is refused for the store, with the path,
/// the cause and the remedy: nothing is committed (no head, no default, no
/// ledger row), the source is not changed, and a repaired file imports.
#[test]
fn a_source_the_strict_reader_refuses_commits_nothing_and_a_repaired_one_imports() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-import-refused");
    let root = root_of(&scratch);
    let scope = Some((289, CONTENT));
    let too_many: Vec<(i32, i32)> = (1..=1025).map(|id| (id, 1)).collect();
    let cases: Vec<(&str, Vec<u8>, FailureReason, &str)> = vec![
        (
            "duplicate",
            br#"{"schema_version":1,"schema_version":1,"profile":"local-289","account":"duplicate","revision":289,"content":"test-content","observed_at_unix":5,"rows":[[995,5]]}"#.to_vec(),
            FailureReason::Schema,
            r#"field "schema_version" is named twice in the top level; edit the file to remove one of them"#,
        ),
        (
            "nested",
            br#"{"schema_version":1,"profile":"local-289","account":"nested","observed_at_unix":5,"rows":[[995,5]],"future":{"same":1,"same":2}}"#.to_vec(),
            FailureReason::Schema,
            r#"field "same" is named twice in the object "future""#,
        ),
        (
            "identity",
            legacy_document("someone", scope, 1, &[(995, 1)], &[]),
            FailureReason::Identity,
            "file names profile",
        ),
        (
            "schema",
            serde_json::to_vec(&json!({
                "schema_version": 7, "profile": PROFILE, "account": "schema",
                "revision": 289, "content": CONTENT, "observed_at_unix": 1, "rows": [[995, 1]],
            }))
            .unwrap(),
            FailureReason::Schema,
            "schema_version 7",
        ),
        (
            "halfscope",
            serde_json::to_vec(&json!({
                "schema_version": 1, "profile": PROFILE, "account": "halfscope",
                "revision": 289, "observed_at_unix": 1, "rows": [[995, 1]],
            }))
            .unwrap(),
            FailureReason::Identity,
            "only one of its two parts",
        ),
        (
            "preschema",
            serde_json::to_vec(&json!({
                "profile": PROFILE, "account": "preschema",
                "observed_at_unix": 1, "rows": [[995, 1]],
            }))
            .unwrap(),
            FailureReason::UnsupportedSavedFormat,
            "from before 0.2.8",
        ),
        (
            "toomany",
            legacy_document("toomany", scope, 1, &too_many, &[]),
            FailureReason::Schema,
            "more than the row cap",
        ),
        (
            "unsorted",
            legacy_document("unsorted", scope, 1, &[(5, 1), (3, 1)], &[]),
            FailureReason::Schema,
            "ids out of order",
        ),
        (
            "corrupt",
            b"{not json".to_vec(),
            FailureReason::Schema,
            "json:",
        ),
    ];
    let notices = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = Arc::clone(&notices);
    let state = State {
        hooks: Hooks {
            read_refusal: Some(Arc::new(move |_, line| {
                sink.lock().unwrap().push(line.to_string());
            })),
            ..Hooks::default()
        },
        ..State::default()
    };
    for (account, bytes, _, _) in &cases {
        write_private(&root.bank_hint_file(PROFILE, account), bytes);
    }
    let service = LocalStateService::open_with_state(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
        state,
    );
    for (account, bytes, reason, cause) in &cases {
        let identity = bank_identity(account);
        let path = root.bank_hint_file(PROFILE, account);
        let refused = refusal_of(*reason);
        assert_eq!(
            service.readback_lease(&identity, WAIT),
            Err(refused),
            "{account}"
        );
        assert_eq!(service.lease(&identity), Err(refused), "{account}");
        assert_eq!(
            service.submit(hint_edit(&identity, None, vec![(995, 99)], 9)),
            SubmitReceipt::Refused(refused),
            "{account}"
        );
        assert_eq!(&std::fs::read(&path).unwrap(), bytes, "{account}: kept");
        assert_eq!(
            siblings(&path),
            vec![path.file_name().unwrap().to_string_lossy().into_owned()],
            "{account}: no evidence or quarantine copy"
        );
        assert_eq!(ledger(&root, account), None, "{account}: no ledger row");
        let path_text = path.display().to_string();
        let notices = notices.lock().unwrap();
        let notice = notices
            .iter()
            .find(|notice| notice.contains(&path_text))
            .unwrap_or_else(|| panic!("{account}: no refusal notice for {path_text}"));
        assert!(notice.contains(cause), "{account}: {cause:?} in {notice}");
        assert!(
            notice.contains("start empty"),
            "{account}: no remedy in {notice}"
        );
        assert!(
            notice.contains("it is neither leased nor written until it reads cleanly"),
            "{account}: {notice}"
        );
    }
    assert!(stored(&root).is_empty(), "no head or default was committed");

    // The operator repairs the file: it is accepted again and imports.
    let repaired = legacy_document("duplicate", scope, 6, &[(995, 6)], &[]);
    write_private(&root.bank_hint_file(PROFILE, "duplicate"), &repaired);
    let lease = service
        .readback_lease(&bank_identity("duplicate"), WAIT)
        .unwrap()
        .unwrap();
    assert_eq!(lease.snapshot().as_bank_hints().unwrap(), &[(995, 6)]);
}

/// A source that is not private (writable by others) is refused for the
/// store with its cause and left alone.
#[cfg(unix)]
#[test]
fn a_source_that_is_not_private_is_refused_and_left_alone() {
    let _state_lease = crate::test_process::state();
    if running_as_root() {
        return;
    }
    let scratch = IsolatedEnv::enter("bank-import-modes");
    let root = root_of(&scratch);
    for (account, mode) in [("modesixsix", 0o666), ("modezero", 0o000)] {
        let path = root.bank_hint_file(PROFILE, account);
        let bytes = legacy_document(account, Some((289, CONTENT)), 1, &[(995, 1)], &[]);
        write_private(&path, &bytes);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        let service = bank_service(&scratch);
        assert_eq!(
            service.readback_lease(&bank_identity(account), WAIT),
            Err(refusal_of(FailureReason::Io)),
            "{account}"
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "{account} kept");
        assert_eq!(ledger(&root, account), None);
    }
}

/// A file with a field this build does not know imports its usable rows and
/// is read-only: saves are refused, also after a restart. The command that
/// accepts the projection asks first, needs the original kept as evidence,
/// and only then lets a save succeed. The original is never changed.
#[test]
fn an_unknown_field_imports_read_only_until_the_operator_accepts_the_projection() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-projection");
    let root = root_of(&scratch);
    let source = root.bank_hint_file(PROFILE, "alice");
    let bytes = legacy_document(
        "alice",
        Some((289, CONTENT)),
        11,
        &[(995, 11)],
        &[("future", json!({"nested": [1, 2]})), ("more", json!(true))],
    );
    write_private(&source, &bytes);
    let alice = bank_identity("alice");
    let key = format!("{PROFILE}/alice/289/{CONTENT}");
    let schema = refusal_of(FailureReason::Schema);

    {
        let service = bank_service(&scratch);
        let lease = service.readback_lease(&alice, WAIT).unwrap().unwrap();
        assert_eq!(lease.snapshot().as_bank_hints().unwrap(), &[(995, 11)]);
        assert_eq!(
            service.submit(hint_edit(&alice, None, vec![(995, 12)], 12)),
            SubmitReceipt::Refused(schema),
            "a read-only projection is never rewritten"
        );
    }
    assert_eq!(stored(&root)[0].5, 2, "both omitted fields are recorded");
    {
        let service = bank_service(&scratch);
        assert!(service.readback_lease(&alice, WAIT).unwrap().is_some());
        assert_eq!(
            service.submit(hint_edit(&alice, None, vec![(995, 12)], 12)),
            SubmitReceipt::Refused(schema),
            "still read-only after a restart"
        );
    }

    let asked = std::sync::Mutex::new(Vec::new());
    let decline = |question: &str| {
        asked.lock().unwrap().push(question.to_string());
        false
    };
    let action = MaintenanceAction::AcceptBankProjection(key.clone());
    let refused = run_maintenance(&root, Frontend::Panel, &action, &decline).unwrap_err();
    assert!(refused.message().contains("Not confirmed"), "{refused}");
    let question = asked.lock().unwrap()[0].clone();
    for part in [&key, "2 omitted field(s)", "1 usable row(s)", ".migrated"] {
        assert!(question.contains(part), "{part} in {question}");
    }
    assert_eq!(
        stored(&root)[0].5,
        2,
        "an unconfirmed command changes nothing"
    );

    // The original must be kept as evidence.
    let evidence = migrated(&source);
    let kept = std::fs::read(&evidence).unwrap();
    std::fs::rename(&evidence, scratch.home.join("away")).unwrap();
    let missing = run_maintenance(&root, Frontend::Panel, &action, &|_| true).unwrap_err();
    assert!(missing.message().contains("not kept"), "{missing}");
    assert_eq!(stored(&root)[0].5, 2);
    std::fs::rename(scratch.home.join("away"), &evidence).unwrap();

    run_maintenance(&root, Frontend::Panel, &action, &|_| true).unwrap();
    assert_eq!(stored(&root)[0].5, 0);
    assert_eq!(
        std::fs::read(&evidence).unwrap(),
        kept,
        "the evidence is untouched"
    );

    let service = bank_service(&scratch);
    let lease = service.readback_lease(&alice, WAIT).unwrap().unwrap();
    assert_eq!(lease.snapshot().as_bank_hints().unwrap(), &[(995, 11)]);
    let saved = request(service.submit(hint_edit(&alice, None, vec![(995, 12)], 12)));
    completed(&service, saved);
    assert_eq!(stored(&root)[0].4, vec![(995, 12)]);
    drop(service);
    let again = run_maintenance(&root, Frontend::Panel, &action, &|_| true).unwrap_err();
    assert!(
        again.message().contains("no read-only projection"),
        "{again}"
    );
}

/// A save is one transaction: it settles with the version the database
/// allocated, survives a restart, and an expected version that lost the race
/// conflicts without changing the stored hint.
#[test]
fn a_save_is_durable_across_a_restart_and_a_stale_version_conflicts() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-save");
    let root = root_of(&scratch);
    let alice = bank_identity("alice");
    let at = 1_759_700_123;
    let first = {
        let service = bank_service(&scratch);
        assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
        let stale = service.lease(&alice).unwrap().version();
        let receipt = completed(
            &service,
            request(service.submit(hint_edit(&alice, None, vec![(379, 12), (995, 3400)], at))),
        );
        assert_eq!(stored(&root)[0].3, at as i64);
        let conflict = request(service.submit(hint_edit(&alice, Some(stale), vec![(1, 1)], at)));
        assert_eq!(
            settlement(&service, conflict),
            Settlement::Failed(ErrorCode::Conflict {
                expected: stale,
                found: receipt.version(),
            })
        );
        receipt.version()
    };
    assert_eq!(stored(&root)[0].4, vec![(379, 12), (995, 3400)]);

    let service = bank_service(&scratch);
    let lease = service.readback_lease(&alice, WAIT).unwrap().unwrap();
    assert_eq!(lease.version(), first);
    assert_eq!(lease.at().unix_seconds(), at);
    assert_eq!(
        lease.snapshot().as_bank_hints().unwrap(),
        &[(379, 12), (995, 3400)]
    );
    let next = completed(
        &service,
        request(service.submit(hint_edit(&alice, Some(first), vec![(995, 1)], at + 1))),
    );
    assert!(next.version() > first, "versions only grow");
}

/// The same rows at the same time complete at the existing version without a
/// commit.
#[test]
fn identical_rows_complete_at_the_existing_version_without_a_commit() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-identical");
    let service = bank_service(&scratch);
    let alice = bank_identity("alice");
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    let audits = audits(&service);
    let first = completed(
        &service,
        request(service.submit(hint_edit(&alice, None, vec![(995, 3400)], 5))),
    );
    let second = completed(
        &service,
        request(service.submit(hint_edit(&alice, None, vec![(995, 3400)], 5))),
    );
    assert_eq!(second.version(), first.version());
    assert_eq!(
        audits.lock().unwrap().len(),
        1,
        "one commit, one audit line"
    );
}

/// A save through another service on the root is an outside change: the
/// other service's save, accepted against what it read before, conflicts and
/// takes the stored rows; its next save goes through, and the first service
/// then conflicts in the same way and holds the outside rows.
#[test]
fn another_services_commit_is_an_outside_change_the_stale_save_conflicts_with() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-outside");
    let root = root_of(&scratch);
    let alice = bank_identity("alice");
    let ours = bank_service(&scratch);
    let theirs = bank_service(&scratch);
    assert_eq!(ours.readback_lease(&alice, WAIT).unwrap(), None);
    assert_eq!(theirs.readback_lease(&alice, WAIT).unwrap(), None);
    let first = completed(
        &ours,
        request(ours.submit(hint_edit(&alice, None, vec![(995, 10)], 10))),
    );

    // Their cache still says no hint: the save is not written over ours.
    let stale = request(theirs.submit(hint_edit(&alice, None, vec![(995, 20)], 20)));
    assert!(
        matches!(
            settlement(&theirs, stale),
            Settlement::Failed(ErrorCode::Conflict { found, .. }) if found == first.version()
        ),
        "the stale save did not fabricate a write over the stored rows"
    );
    assert_eq!(stored(&root)[0].4, vec![(995, 10)]);
    let lease = theirs.lease(&alice).unwrap();
    assert_eq!(lease.snapshot().as_bank_hints().unwrap(), &[(995, 10)]);
    assert_eq!(lease.version(), first.version());

    // Having taken the stored rows, their next save goes through.
    let outside = completed(
        &theirs,
        request(theirs.submit(hint_edit(&alice, None, vec![(995, 20)], 20))),
    );
    assert!(outside.version() > first.version());

    let ten = request(ours.submit(hint_edit(&alice, None, vec![(995, 10)], 10)));
    assert!(
        matches!(
            settlement(&ours, ten),
            Settlement::Failed(ErrorCode::Conflict { found, .. }) if found == outside.version()
        ),
        "the resubmission did not fabricate a write over the outside rows"
    );
    assert_eq!(stored(&root)[0].4, vec![(995, 20)]);
    let lease = ours.lease(&alice).unwrap();
    assert_eq!(lease.snapshot().as_bank_hints().unwrap(), &[(995, 20)]);
    assert_eq!(lease.version(), outside.version());
}

/// A new queued hint supersedes the old one, which follows it to
/// durability: only the newest is committed.
#[test]
fn a_new_queued_hint_supersedes_the_old_one_which_follows_it_to_durability() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-superseded");
    let root = root_of(&scratch);
    let service = bank_service(&scratch);
    let alice = bank_identity("alice");
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    let audits = audits(&service);
    // Hold the writer inside the first save until both hints are queued.
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    let once = AtomicBool::new(false);
    service.set_write_stall_for_test(Arc::new(move || {
        if !once.swap(true, Ordering::SeqCst) {
            let _ = entered_tx.send(());
            let _ = release_rx.lock().unwrap().recv();
        }
    }));

    let held = request(service.submit(hint_edit(&alice, None, vec![(995, 1)], 100)));
    entered_rx.recv_timeout(WAIT).unwrap();
    let first = request(service.submit(hint_edit(&alice, None, vec![(995, 2)], 101)));
    let second = request(service.submit(hint_edit(&alice, None, vec![(995, 3)], 102)));
    assert_eq!(
        service.observe(first),
        Some(Settlement::Cancelled(Cancellation::Superseded))
    );
    assert_eq!(
        service.wait_durable(first, Duration::from_millis(50)),
        Durability::Pending,
        "a superseded hint is durable only when its replacement is"
    );
    release_tx.send(()).unwrap();
    completed(&service, held);
    let durable = service.wait_durable(first, WAIT);
    assert!(
        matches!(&durable, Durability::Durable(receipt) if receipt.request() == Some(second)),
        "{durable:?}"
    );
    assert_eq!(stored(&root)[0].4, vec![(995, 3)]);
    assert_eq!(
        audits.lock().unwrap().len(),
        2,
        "the held save and the newest"
    );
}

#[test]
fn a_waiting_bank_write_is_superseded_before_it_can_retry() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-superseded-sql-retry");
    let root = root_of(&scratch);
    let service = bank_service(&scratch);
    let alice = bank_identity("alice");
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    let holder = inspect(&root);
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let release_rx = std::sync::Mutex::new(release_rx);
    let first = AtomicBool::new(true);
    service.set_write_stall_for_test(Arc::new(move || {
        if first.swap(false, Ordering::SeqCst) {
            let _ = entered_tx.send(());
            let _ = release_rx.lock().unwrap().recv();
        }
    }));

    let old = request(service.submit(hint_edit(&alice, None, vec![(995, 7)], 100)));
    entered.recv_timeout(WAIT).unwrap();
    let new = request(service.submit(hint_edit(&alice, None, vec![(995, 9)], 100)));
    release.send(()).unwrap();
    let while_locked = settled_within(&service, old, Duration::from_secs(2));
    holder.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        while_locked,
        Some(Settlement::Cancelled(Cancellation::Superseded))
    );
    assert!(matches!(
        service.wait_durable(old, WAIT),
        Durability::Durable(receipt) if receipt.request() == Some(new)
    ));
    assert_eq!(stored(&root)[0].4, vec![(995, 9)]);
}

#[test]
fn bank_observation_order_and_head_survive_store_retirement() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-observation-retirement");
    let root = root_of(&scratch);
    let service = bank_service(&scratch);
    let alice = bank_identity("alice");
    let binding = BankHintBinding::new(alice.clone());
    let hint = |rows, seconds| {
        BankHint::try_new(
            alice.clone(),
            None,
            vec![(995, rows)],
            DurableTimestamp::from_unix_seconds(seconds),
        )
        .unwrap()
    };
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    let old = service.stamp_bank_observation(&binding);
    let old_attempt = request(service.submit_bank(&binding, hint(7, 100), old));
    completed(&service, old_attempt);
    let observation = service.stamp_bank_observation(&binding);
    let newest = request(service.submit_bank(&binding, hint(9, 100), observation));
    let head = completed(&service, newest).version();
    // Retire the resident store through the production eviction path.
    service.retire_idle_bank_for_test(&binding);
    assert_eq!(service.request_state(newest), RequestState::Forgotten);
    assert!(service.is_bank_attempt(&binding, newest));
    assert!(!service.is_bank_attempt(&binding, old_attempt));
    // New commit order or a later wall clock must not make the older sample newer.
    assert!(matches!(
        service.submit_bank(&binding, hint(7, 101), old),
        SubmitReceipt::Refused(_)
    ));
    assert_eq!(stored(&root)[0].4, vec![(995, 9)]);

    let observed_before_other_writer = service.stamp_bank_observation(&binding);
    let other = bank_service(&scratch);
    assert!(other.readback_lease(&alice, WAIT).unwrap().is_some());
    let newer_head = completed(
        &other,
        request(other.submit(hint_edit(&alice, Some(head), vec![(995, 11)], 100))),
    )
    .version();
    assert_eq!(
        service
            .readback_lease(&alice, WAIT)
            .unwrap()
            .unwrap()
            .version(),
        newer_head
    );
    assert_eq!(
        service.submit_bank(&binding, hint(10, 100), observed_before_other_writer),
        SubmitReceipt::Refused(ErrorCode::Conflict {
            expected: head,
            found: newer_head
        })
    );
    assert_eq!(stored(&root)[0].4, vec![(995, 11)]);
}

/// A failed save keeps the stored hint and a following submission succeeds.
#[test]
fn a_failed_save_keeps_the_stored_hint_and_a_following_save_can_succeed() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-failed-save");
    let root = root_of(&scratch);
    let service = bank_service(&scratch);
    let alice = bank_identity("alice");
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    completed(
        &service,
        request(service.submit(hint_edit(&alice, None, vec![(995, 20)], 200))),
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    service.set_write_fault_for_test(Arc::new(move || {
        counted.fetch_add(1, Ordering::SeqCst) == 0
    }));

    let failed = request(service.submit(hint_edit(&alice, None, vec![(995, 25)], 201)));
    assert_eq!(
        settlement(&service, failed),
        Settlement::Failed(refusal_of(FailureReason::Io))
    );
    assert_eq!(stored(&root)[0].4, vec![(995, 20)]);
    let retry = request(service.submit(hint_edit(&alice, None, vec![(995, 25)], 201)));
    completed(&service, retry);
    assert_eq!(stored(&root)[0].4, vec![(995, 25)]);
}

/// The database commit faults the preferences settle under settle the same
/// way for a bank save: a refused COMMIT fails it and keeps the hint; one
/// that was applied and then reported as a disk error is found by its
/// recent-commit row and completes.
#[test]
fn bank_saves_settle_commit_faults_like_the_preferences_do() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-commit-faults");
    let root = root_of(&scratch);
    let service = bank_service(&scratch);
    let alice = bank_identity("alice");
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    completed(
        &service,
        request(service.submit(hint_edit(&alice, None, vec![(995, 1)], 1))),
    );
    let fault = Arc::new(std::sync::Mutex::new(Some(sql::CommitFault::Full)));
    let taken = Arc::clone(&fault);
    service.set_sql_seams_for_test(sql::Seams {
        commit_fault: Some(Arc::new(move || taken.lock().unwrap().take())),
        ..sql::Seams::default()
    });
    let refused = request(service.submit(hint_edit(&alice, None, vec![(995, 2)], 2)));
    assert_eq!(
        settlement(&service, refused),
        Settlement::Failed(refusal_of(FailureReason::Io))
    );
    assert_eq!(stored(&root)[0].4, vec![(995, 1)], "the hint is kept");

    *fault.lock().unwrap() = Some(sql::CommitFault::IoErrAfterCommit);
    let recovered = request(service.submit(hint_edit(&alice, None, vec![(995, 3)], 3)));
    let receipt = completed(&service, recovered);
    assert_eq!(stored(&root)[0].4, vec![(995, 3)]);
    assert_eq!(
        service.lease(&alice).unwrap().version(),
        receipt.version(),
        "the recovered COMMIT is the version the service holds"
    );
}

/// The import commit and the evidence move are separate: a process that
/// stops between them leaves the ledger `pending`, and the next readback
/// finishes the move without applying the source again.
#[test]
fn an_import_interrupted_before_its_evidence_is_finished_by_the_next_read() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-import-pending");
    let root = root_of(&scratch);
    let source = root.bank_hint_file(PROFILE, "alice");
    let bytes = legacy_document("alice", Some((289, CONTENT)), 4, &[(995, 4)], &[]);
    write_private(&source, &bytes);
    let alice = bank_identity("alice");
    {
        let service = bank_service(&scratch);
        // The writer stops right after the import committed.
        service.set_sql_seams_for_test(sql::Seams {
            point: Some(Arc::new(|name| {
                if name == "after-bank-import-commit" {
                    panic!("test: the process stops after the import commit");
                }
            })),
            ..sql::Seams::default()
        });
        let _ = service.readback_lease(&alice, Duration::from_millis(500));
    }
    assert_eq!(
        ledger(&root, "alice"),
        Some(("imported".into(), "pending".into()))
    );
    assert!(source.exists(), "the evidence move did not run");
    assert_eq!(stored(&root).len(), 1, "the import itself committed");

    let service = bank_service(&scratch);
    let lease = service.readback_lease(&alice, WAIT).unwrap().unwrap();
    assert_eq!(lease.snapshot().as_bank_hints().unwrap(), &[(995, 4)]);
    assert!(!source.exists());
    assert_eq!(std::fs::read(migrated(&source)).unwrap(), bytes);
    assert_eq!(
        ledger(&root, "alice"),
        Some(("imported".into(), "completed".into()))
    );
    assert_eq!(stored(&root).len(), 1, "the source was not applied twice");
}

fn fill_bank(root: &StateRoot, hints: u64, rows_each: usize) {
    let conn = inspect(root);
    let rows: Vec<u8> = (1..=rows_each as i32)
        .flat_map(|id| {
            let mut row = id.to_be_bytes().to_vec();
            row.extend_from_slice(&1_i32.to_be_bytes());
            row
        })
        .collect();
    conn.execute_batch("BEGIN").unwrap();
    for n in 0..hints {
        conn.execute(
            "INSERT INTO bank_hint
                 (profile, account, revision, content, version, observed_at, rows, omitted)
             VALUES (?1, ?2, 289, ?3, ?4, 1, ?5, 0)",
            params![
                format!("filler-{}", n / 100),
                format!("f{}", n % 100),
                CONTENT,
                (n + 1_000_000).to_be_bytes().to_vec(),
                rows
            ],
        )
        .unwrap();
    }
    conn.execute_batch("COMMIT").unwrap();
}

/// The database holds at most `MAX_BANK_HINTS` hints and never evicts one: a
/// new identity is refused `Busy`, an import of a new source is refused with
/// its cause and keeps its file, and a hint that is stored still saves.
#[test]
fn the_database_never_holds_more_than_its_hint_cap_and_evicts_none() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-cap");
    let root = root_of(&scratch);
    let service = bank_service(&scratch);
    let alice = bank_identity("alice");
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    completed(
        &service,
        request(service.submit(hint_edit(&alice, None, vec![(995, 1)], 1))),
    );
    fill_bank(&root, sql::MAX_BANK_HINTS - 1, 8);
    assert_eq!(stored(&root).len() as u64, sql::MAX_BANK_HINTS);

    let bob = bank_identity("bob");
    assert_eq!(service.readback_lease(&bob, WAIT).unwrap(), None);
    let refused = request(service.submit(hint_edit(&bob, None, vec![(995, 2)], 2)));
    assert_eq!(
        settlement(&service, refused),
        Settlement::Failed(ErrorCode::Busy {
            domain: LocalStateDomain::BankHints
        })
    );
    assert_eq!(stored(&root).len() as u64, sql::MAX_BANK_HINTS);

    // A stored hint still saves.
    completed(
        &service,
        request(service.submit(hint_edit(&alice, None, vec![(995, 3)], 3))),
    );

    // A source that would need a new hint is refused, and kept.
    let carol_path = root.bank_hint_file(PROFILE, "carol");
    let carol = legacy_document("carol", Some((289, CONTENT)), 1, &[(995, 1)], &[]);
    write_private(&carol_path, &carol);
    assert_eq!(
        service.readback_lease(&bank_identity("carol"), WAIT),
        Err(refusal_of(FailureReason::Io))
    );
    assert_eq!(std::fs::read(&carol_path).unwrap(), carol);
    assert_eq!(ledger(&root, "carol"), None);
}

/// The page budget: a database with every bank hint at its widest holds the
/// largest preference document and rewrites it, inside the page cap and the
/// WAL bound, without SQLITE_FULL.
#[test]
fn a_full_bank_and_the_largest_preferences_fit_the_page_cap() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-pages");
    let root = root_of(&scratch);
    let service = LocalStateService::open_process(root.clone());
    let preferences = service.preferences();
    let deadline = Instant::now() + WAIT;
    while preferences.snapshot().is_err() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    fill_bank(&root, sql::MAX_BANK_HINTS, api::bank_memory::MAX_HINT_ROWS);
    let pages: i64 = inspect(&root)
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .unwrap();
    assert!(
        pages as u64 <= sql::BANK_PAGES + 8,
        "a full bank takes {pages} pages, within its {} pages",
        sql::BANK_PAGES
    );

    // A preference document near its 1 MiB bound, member by member.
    let chunk = "x".repeat(900);
    for n in 0..1000 {
        let _ = preferences.begin_set(&format!("pad-{n:04}"), json!(chunk));
        if n % 50 == 49 {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let alice = bank_identity("alice");
    let _ = service.readback_lease(&alice, WAIT);
    let total: i64 = inspect(&root)
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .unwrap();
    assert!(
        total as u64 <= sql::DB_MAX_PAGES,
        "{total} pages within the {} page cap",
        sql::DB_MAX_PAGES
    );
}

/// A version-1 database (preferences only, the one-row ledger) moves to
/// the current schema at its first open: the preferences ledger row is kept as it was,
/// the bank tables appear, and nothing is imported again.
#[test]
fn a_version_1_database_moves_to_the_current_schema_keeping_the_completed_ledger() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-schema-step");
    let root = root_of(&scratch);
    {
        // The service creates the current schema; rebuild version 1 from it.
        let service = LocalStateService::open_process(root.clone());
        let preferences = service.preferences();
        let deadline = Instant::now() + WAIT;
        while preferences.snapshot().is_err() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let conn = inspect(&root);
    let before: (String, String) = conn
        .query_row(
            "SELECT kind, evidence FROM import_ledger WHERE domain = 'preferences'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    conn.execute_batch(
        "CREATE TABLE legacy_source (
             id INTEGER PRIMARY KEY CHECK (id = 1),
             digest BLOB CHECK (digest IS NULL OR length(digest) = 32),
             kind TEXT NOT NULL,
             evidence TEXT NOT NULL,
             path TEXT NOT NULL CHECK (length(path) <= 4096)
         ) STRICT;
         INSERT INTO legacy_source (id, digest, kind, evidence, path)
             SELECT 1, digest, kind, evidence, path FROM import_ledger
             WHERE domain = 'preferences';
         DROP TABLE import_ledger;
         DROP TABLE bank_hint;
         DROP TABLE operator_script_card;
         DROP TABLE operator_script_head;
         DROP TABLE script_runtime_settings;
         DROP TABLE server_world;
         DROP TABLE server_profile;
         PRAGMA user_version = 1;",
    )
    .unwrap();
    drop(conn);

    let service = LocalStateService::open_process(root.clone());
    let preferences = service.preferences();
    let deadline = Instant::now() + WAIT;
    while preferences.snapshot().is_err() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let conn = inspect(&root);
    let version: i32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, sql::USER_VERSION);
    let after: (String, String) = conn
        .query_row(
            "SELECT kind, evidence FROM import_ledger
             WHERE domain = 'preferences' AND source = 'panel-ui.json'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(after, before, "the completed entry is preserved");
    let old: Option<i64> = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name = 'legacy_source'",
            [],
            |row| row.get(0),
        )
        .optional()
        .unwrap();
    assert_eq!(old, Some(0));
    let alice = bank_identity("alice");
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
}

/// One writer thread opens one connection: the bank hints and the
/// preferences of one service share it, and a `Play`'s composition over an
/// injected process service opens no provisional service of its own.
#[test]
fn an_injected_process_service_is_the_only_one_a_play_composes_over() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-one-connection");
    let root = root_of(&scratch);
    let process = LocalStateService::open_process(root.clone());
    let opened_before = PROVISIONAL_OPENED.load(Ordering::Relaxed);
    let scope = ContentScope::try_new(client::io::ClientRevision::R289, CONTENT).unwrap();
    let composition = StateComposition::for_play(
        root.path(),
        PROFILE,
        scope.clone(),
        Some(Arc::clone(&process)),
    );
    assert!(Arc::ptr_eq(composition.service(), &process));
    assert_eq!(
        PROVISIONAL_OPENED.load(Ordering::Relaxed),
        opened_before,
        "an injected service opens no provisional one"
    );
    let alice = composition
        .identity(LocalStateDomain::BankHints, "alice")
        .unwrap();
    assert_eq!(
        composition.service().readback_lease(&alice, WAIT).unwrap(),
        None
    );
    assert_eq!(
        sql::connections_opened_for_test(&root),
        1,
        "the bank hints and the preferences share one connection"
    );

    let own = StateComposition::for_play(root.path(), PROFILE, scope, None);
    assert!(!Arc::ptr_eq(own.service(), &process));
    assert_eq!(
        PROVISIONAL_OPENED.load(Ordering::Relaxed),
        opened_before + 1,
        "without an injected service the Play opens a provisional one"
    );
}

/// A held product guard asks a bank job to wait, as it does a preference
/// save: the job is retried with delays, the bank wait never shows as a
/// preference wait, and the job completes once the guard is free.
#[test]
fn a_held_guard_makes_bank_jobs_wait_without_a_preference_wait() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-guard-wait");
    let root = root_of(&scratch);
    let service = bank_service(&scratch);
    let alice = bank_identity("alice");
    std::fs::create_dir_all(root.path()).unwrap();
    let guard = files::try_lock_for_write(&root.preferences_file())
        .unwrap()
        .expect("the guard is free");
    let reader = {
        let service = Arc::clone(&service);
        let alice = alice.clone();
        std::thread::spawn(move || service.readback_lease(&alice, WAIT))
    };
    std::thread::sleep(Duration::from_millis(200));
    assert!(!reader.is_finished(), "the readback waits for the guard");
    assert_eq!(
        service.shared.state.lock().preference_wait,
        None,
        "a bank wait is not a preference wait"
    );
    drop(guard);
    assert_eq!(reader.join().unwrap().unwrap(), None);
    let saved = request(service.submit(hint_edit(&alice, None, vec![(995, 1)], 1)));
    completed(&service, saved);
}

/// A preferences recovery keeps the bank hints and their import ledger: the
/// hints are carried into the new history unchanged, and a recovery that
/// cannot read the old database records that no legacy hint file is
/// imported again.
#[test]
fn a_preferences_recovery_keeps_the_bank_hints() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-recovery");
    let root = root_of(&scratch);
    let alice = bank_identity("alice");
    {
        let service = LocalStateService::open_process(root.clone());
        let preferences = service.preferences();
        let deadline = Instant::now() + WAIT;
        while preferences.snapshot().is_err() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
        completed(
            &service,
            request(service.submit(hint_edit(&alice, None, vec![(995, 7)], 7))),
        );
    }
    let report =
        run_preferences_maintenance(&root, Frontend::Panel, &MaintenanceAction::RecoverDefaults)
            .unwrap();
    assert!(
        report.contains("1 bank hint(s) were carried over"),
        "{report}"
    );
    assert_eq!(stored(&root)[0].4, vec![(995, 7)]);
    assert_eq!(
        ledger(&root, "alice"),
        Some(("absent".into(), "none".into())),
        "the ledger is carried with the hint"
    );
    let service = bank_service(&scratch);
    let lease = service.readback_lease(&alice, WAIT).unwrap().unwrap();
    assert_eq!(lease.snapshot().as_bank_hints().unwrap(), &[(995, 7)]);

    // An old database that cannot be read: its hints stay in the preserved
    // set, and no legacy file is offered for an automatic import.
    drop(service);
    std::fs::write(root.database_file(), b"this is not a database").unwrap();
    let bob_path = root.bank_hint_file(PROFILE, "bob");
    write_private(
        &bob_path,
        &legacy_document("bob", Some((289, CONTENT)), 1, &[(995, 1)], &[]),
    );
    let report =
        run_preferences_maintenance(&root, Frontend::Panel, &MaintenanceAction::RecoverDefaults)
            .unwrap();
    assert!(
        report.contains("no legacy bank hint file is imported again"),
        "{report}"
    );
    let service = bank_service(&scratch);
    assert_eq!(
        service.readback_lease(&bank_identity("bob"), WAIT).unwrap(),
        None
    );
    assert!(bob_path.exists(), "the file is left where it is");
}

/// Hints of two content scopes of one account are two hints: neither is
/// leased under the other, and each saves and survives a restart on its own.
#[test]
fn hints_of_two_scopes_of_one_account_are_independent() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-two-scopes");
    let root = root_of(&scratch);
    let under_a = scoped(LocalStateDomain::BankHints, "alice", "content-a");
    let under_b = scoped(LocalStateDomain::BankHints, "alice", "content-b");
    {
        let service = bank_service(&scratch);
        assert_eq!(service.readback_lease(&under_a, WAIT).unwrap(), None);
        assert_eq!(service.readback_lease(&under_b, WAIT).unwrap(), None);
        completed(
            &service,
            request(service.submit(hint_edit(&under_a, None, vec![(995, 7)], 7))),
        );
        assert_eq!(
            service
                .lease(&under_b)
                .unwrap()
                .snapshot()
                .as_bank_hints()
                .unwrap(),
            &[],
            "rows observed under one content scope are never leased under another"
        );
        completed(
            &service,
            request(service.submit(hint_edit(&under_b, None, vec![(995, 8)], 8))),
        );
    }
    let stored = stored(&root);
    assert_eq!(stored.len(), 2);
    assert_eq!(
        (stored[0].2.as_str(), stored[0].4.clone()),
        ("content-a", vec![(995, 7)])
    );
    assert_eq!(
        (stored[1].2.as_str(), stored[1].4.clone()),
        ("content-b", vec![(995, 8)])
    );
    let service = bank_service(&scratch);
    let lease_a = service.readback_lease(&under_a, WAIT).unwrap().unwrap();
    assert_eq!(lease_a.identity(), &under_a);
    assert_eq!(lease_a.snapshot().as_bank_hints().unwrap(), &[(995, 7)]);
    let lease_b = service.readback_lease(&under_b, WAIT).unwrap().unwrap();
    assert_eq!(lease_b.snapshot().as_bank_hints().unwrap(), &[(995, 8)]);
}

/// Nothing is acknowledged before the COMMIT: the request is pending while
/// its transaction is open and no other connection sees the rows; the
/// receipt's version is the stored version, and the one audit line follows
/// the commit and names it.
#[test]
fn the_receipt_and_audit_follow_the_commit() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-commit-order");
    let root = root_of(&scratch);
    let service = bank_service(&scratch);
    let alice = bank_identity("alice");
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    service.set_sql_seams_for_test(sql::Seams {
        point: Some(Arc::new(move |name| {
            if name == "before-commit" {
                let _ = entered_tx.send(());
                let _ = release_rx.lock().unwrap().recv();
            }
        })),
        ..sql::Seams::default()
    });
    let audits = audits(&service);
    let write = request(service.submit(hint_edit(&alice, None, vec![(995, 1)], 1)));
    entered.recv_timeout(WAIT).unwrap();
    assert_eq!(service.request_state(write), RequestState::Pending);
    assert!(
        stored(&root).is_empty(),
        "no other connection sees the rows yet"
    );
    assert!(audits.lock().unwrap().is_empty());
    release.send(()).unwrap();
    let receipt = completed(&service, write);
    let audits = audits.lock().unwrap();
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].version(), receipt.version());
    assert_eq!(audits[0].action(), CommitAction::BankHintWrite);
    let version: Vec<u8> = inspect(&root)
        .query_row("SELECT version FROM bank_hint", [], |row| row.get(0))
        .unwrap();
    assert_eq!(sql::decode_u64(&version), Some(receipt.version().get()));
}

/// A schema-1 hint file at the 64 KiB read ceiling: `rows` rows, padded with
/// JSON whitespace (not with unknown fields, which would make it read-only)
/// up to `size` bytes.
fn ceiling_document(
    account: &str,
    scope: Option<(i32, &str)>,
    rows: usize,
    size: usize,
) -> Vec<u8> {
    let rows: Vec<(i32, i32)> = (0..rows as i32)
        .map(|id| (i32::MAX - 1_100 + id, i32::MAX))
        .collect();
    let compact = legacy_document(account, scope, 1_759_700_123, &rows, &[]);
    assert!(
        compact.len() <= size,
        "{} rows exceed the ceiling",
        rows.len()
    );
    let mut padded = compact;
    padded.pop(); // the closing brace
    padded.extend(std::iter::repeat_n(b' ', size - padded.len() - 1));
    padded.push(b'}');
    padded
}

fn bank_hint_at_width(identity: &DomainIdentity, rows: usize, at: u64) -> DomainEdit {
    let rows: Vec<(i32, i32)> = (0..rows as i32).map(|id| (id + 1, i32::MAX)).collect();
    hint_edit(identity, None, rows, at)
}

/// The bank hint budget of SD1 §7 (S5a): the 64 KiB source ceiling with
/// 1,024 rows, both scope forms, the 256-store resident cap fully occupied,
/// a queued and an in-flight replacement, a held hydration and the
/// observation and exit saves, each job measured whole. Writes
/// `S5A_BUDGET_OUT` (JSON) and fails when a class passes 250 ms.
#[test]
#[ignore = "the bank budget measurement; run alone in a release build with S5A_BUDGET_OUT"]
fn bank_budget_report() {
    use crate::local_state::occupancy_tests::{record, summarize, worst_ms, Samples, THRESHOLD_MS};
    let out = PathBuf::from(std::env::var_os("S5A_BUDGET_OUT").expect("S5A_BUDGET_OUT unset"));
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-budget");
    let root = root_of(&scratch);
    let accounts = MAX_BANK_STORES;
    for n in 0..accounts {
        let name = format!("b{n:03}");
        let scope = (n % 2 == 0).then_some((289, CONTENT));
        write_private(
            &root.bank_hint_file(PROFILE, &name),
            &ceiling_document(&name, scope, 1_024, MAX_HINT_BYTES as usize - 16),
        );
    }
    let service = LocalStateService::open_process(root.clone());
    let jobs = Arc::new(std::sync::Mutex::new(Samples::new()));
    let sink = Arc::clone(&jobs);
    service.set_job_hook_for_test(Arc::new(move |class, took| {
        record(&mut sink.lock().unwrap(), class, took);
    }));
    let mut residence = Samples::new();

    // The first import of every account: parse the ceiling file, store the
    // widest hint, keep the evidence, close the ledger.
    let imports_started = Instant::now();
    for n in 0..accounts {
        let started = Instant::now();
        let lease = service
            .readback_lease(&bank_identity(&format!("b{n:03}")), WAIT)
            .unwrap()
            .expect("the import stored the hint");
        record(&mut residence, "bank-import-residence", started.elapsed());
        assert_eq!(lease.snapshot().as_bank_hints().unwrap().len(), 1_024);
    }
    let all_imports = imports_started.elapsed();
    let resident_banks = service
        .shared
        .state
        .lock()
        .stores
        .keys()
        .filter(|key| matches!(key, StoreKey::BankHint { .. }))
        .count();
    assert_eq!(resident_banks, accounts, "the resident cap is full");

    // A held in-flight save with a replacement queued behind it for every
    // account: the writer holds the first at its start while 255 more and a
    // replacement of each are submitted.
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    let once = AtomicBool::new(false);
    service.set_write_stall_for_test(Arc::new(move || {
        if !once.swap(true, Ordering::SeqCst) {
            let _ = entered_tx.send(());
            let _ = release_rx.lock().unwrap().recv();
        }
    }));
    let at = 1_760_000_000;
    let mut requests = Vec::new();
    let submitted = Instant::now();
    for n in 0..accounts {
        let identity = bank_identity(&format!("b{n:03}"));
        requests.push((
            Instant::now(),
            request(service.submit(bank_hint_at_width(&identity, 1_024, at + n as u64))),
        ));
        if n == 0 {
            entered.recv_timeout(WAIT).unwrap();
        }
    }
    let mut replacements = Vec::new();
    for n in 0..accounts {
        let identity = bank_identity(&format!("b{n:03}"));
        replacements.push(request(service.submit(bank_hint_at_width(
            &identity,
            1_000,
            at + 1_000 + n as u64,
        ))));
    }
    release.send(()).unwrap();
    for (started, request) in &requests {
        let _ = settlement(&service, *request);
        record(&mut residence, "bank-save-residence", started.elapsed());
    }
    for request in &replacements {
        let _ = settlement(&service, *request);
    }
    let drained = submitted.elapsed();

    // Observation and exit saves beside loads of cold identities past the
    // cap (each evicts an idle store and reads the database).
    let observation = bank_identity("b000");
    std::thread::scope(|scope| {
        let exit = scope.spawn(|| {
            let started = Instant::now();
            // The cold loads may evict the observed account's store first; an
            // exit save hydrates it again, as a slot does.
            let receipt = loop {
                match service.submit(bank_hint_at_width(&observation, 1_024, at + 9_000)) {
                    SubmitReceipt::Refused(ErrorCode::NotReady { .. }) => {
                        service.readback_lease(&observation, WAIT).unwrap();
                    }
                    receipt => break receipt,
                }
            };
            completed(&service, request(receipt));
            started.elapsed()
        });
        let cold = scope.spawn(|| {
            let mut took = Vec::new();
            for n in 0..44 {
                let started = Instant::now();
                service
                    .readback_lease(&bank_identity(&format!("c{n:03}")), WAIT)
                    .unwrap();
                took.push(started.elapsed());
            }
            took
        });
        record(
            &mut residence,
            "bank-exit-save-residence",
            exit.join().unwrap(),
        );
        for took in cold.join().unwrap() {
            record(&mut residence, "bank-cold-load-residence", took);
        }
    });
    let evicted = bank_identity("b001");
    let started = Instant::now();
    service.readback_lease(&evicted, WAIT).unwrap().unwrap();
    record(
        &mut residence,
        "bank-evicted-reload-residence",
        started.elapsed(),
    );

    let conn = inspect(&root);
    let pages: i64 = conn
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .unwrap();
    let freelist: i64 = conn
        .query_row("PRAGMA freelist_count", [], |row| row.get(0))
        .unwrap();
    let wal_bytes = std::fs::metadata(format!("{}-wal", root.database_file().display()))
        .map_or(0, |meta| meta.len());
    let mut classes = jobs.lock().unwrap().clone();
    classes.extend(residence);
    let worst = worst_ms(&classes);
    let document = json!({
        "domain": "bank-hints",
        "threshold_ms": THRESHOLD_MS,
        "worst_ms": worst,
        "os": std::env::consts::OS,
        "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "fixture": {
            "accounts": accounts,
            "source_bytes": MAX_HINT_BYTES - 16,
            "rows_per_hint": 1_024,
            "scope_forms": "even accounts scoped, odd accounts unscoped",
            "resident_store_cap": MAX_BANK_STORES,
            "queued_replacements": accounts,
            "in_flight_held": true,
            "cold_loads_past_cap": 44,
        },
        "drain_of_all_saves_ms": drained.as_secs_f64() * 1000.0,
        "first_import_of_all_accounts_ms": all_imports.as_secs_f64() * 1000.0,
        "database": {
            "page_count": pages,
            "freelist_pages": freelist,
            "bank_pages_cap": sql::BANK_PAGES,
            "db_pages_cap": sql::DB_MAX_PAGES,
            "wal_bytes": wal_bytes,
            "sqlite_heap_highwater_bytes": unsafe_sqlite_highwater(),
        },
        "classes": summarize(&classes),
    });
    std::fs::write(&out, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
    assert!(worst <= THRESHOLD_MS, "a class passed 250 ms: {document}");
}

/// The process-wide SQLite heap high-water mark.
fn unsafe_sqlite_highwater() -> i64 {
    // SAFETY: `sqlite3_memory_highwater` reads a counter and has no
    // preconditions; zero keeps the mark.
    unsafe { rusqlite::ffi::sqlite3_memory_highwater(0) }
}

/// Every shape the first import takes, run end to end with a restart after
/// each: what the source holds, when it converts, the version it committed
/// at, where its evidence is, and what a restart reads. Writes
/// `S5A_CONVERSION_OUT` (JSON; no payload of a real operator root is in it).
#[test]
#[ignore = "the conversion evidence; run alone with S5A_CONVERSION_OUT"]
fn bank_conversion_report() {
    let out =
        PathBuf::from(std::env::var_os("S5A_CONVERSION_OUT").expect("S5A_CONVERSION_OUT unset"));
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("bank-conversion");
    let root = root_of(&scratch);
    let rows = [(379, 12), (995, 3400)];
    let shapes: Vec<(&str, &str, Option<Vec<u8>>, &str)> = vec![
        ("scoped-schema-1", "alice", Some(legacy_document("alice", Some((289, CONTENT)), 11, &rows, &[])), "0.2.9 file with its recorded scope"),
        ("unscoped-schema-1", "bob", Some(legacy_document("bob", None, 12, &rows, &[])), "0.2.8 file without a scope"),
        ("absent", "carol", None, "no file"),
        ("other-scope", "dave", Some(legacy_document("dave", Some((289, "content-elsewhere")), 13, &rows, &[])), "a scope this launch does not have"),
        ("duplicate-field", "erin", Some(br#"{"schema_version":1,"schema_version":1,"profile":"local-289","account":"erin","observed_at_unix":1,"rows":[]}"#.to_vec()), "refused, kept"),
        ("pre-schema", "frank", Some(serde_json::to_vec(&json!({"profile": PROFILE, "account": "frank", "observed_at_unix": 1, "rows": []})).unwrap()), "refused, unsupported format"),
        ("unknown-field", "gina", Some(legacy_document("gina", Some((289, CONTENT)), 14, &rows, &[("future", json!(1))])), "imported read-only"),
    ];
    for (_, account, bytes, _) in &shapes {
        if let Some(bytes) = bytes {
            write_private(&root.bank_hint_file(PROFILE, account), bytes);
        }
    }
    let mut report = Vec::new();
    for (shape, account, _, note) in &shapes {
        let source = root.bank_hint_file(PROFILE, account);
        let identity = bank_identity(account);
        let first = {
            let service = bank_service(&scratch);
            service.readback_lease(&identity, WAIT)
        };
        let restart = {
            let service = bank_service(&scratch);
            service.readback_lease(&identity, WAIT)
        };
        let summarize = |result: &Result<Option<DomainLease>, ErrorCode>| match result {
            Ok(Some(lease)) => json!({
                "version": lease.version().get(),
                "rows": lease.snapshot().as_bank_hints().unwrap().len(),
                "observed_at_unix": lease.at().unix_seconds(),
            }),
            Ok(None) => json!("empty"),
            Err(code) => json!({ "refused": code.to_string() }),
        };
        report.push(json!({
            "shape": shape,
            "note": note,
            "source": format!("bank-hints/{PROFILE}/{account}.json"),
            "conversion_moment": "first readback of the account for its launch scope",
            "first_read": summarize(&first),
            "restart_read": summarize(&restart),
            "ledger": ledger(&root, account),
            "evidence": migrated(&source).exists().then(|| format!("{account}.json.migrated")),
            "source_left_in_place": source.exists(),
        }));
    }
    std::fs::write(
        &out,
        serde_json::to_vec_pretty(&json!({ "domain": "bank-hints", "shapes": report })).unwrap(),
    )
    .unwrap();
}
