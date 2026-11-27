//! S6a loadout persistence tests use private files and real SQLite stores.

use super::*;
use api::identity::Settlement;
use api::loadout::{CarryEntry, Loadout};
use api::local_state::{LoadoutEdit, LoadoutKey};
use crate::local_state::LoadoutSnapshot;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const LOADOUT_WAIT: Duration = Duration::from_secs(60);
const OPERATOR_STORE: &str = "operator-presets";

fn operator_identity() -> DomainIdentity {
    DomainIdentity::new(
        LocalStateDomain::LoadoutsReferences,
        AccountKey::try_new("operator", "operator").unwrap(),
        ContentScope::try_new(client::io::ClientRevision::R289, "operator").unwrap(),
    )
}

fn account_context(name: &str) -> (LoadoutKey, DomainIdentity) {
    let account = AccountKey::try_new("local-289", name).unwrap();
    let scope = ContentScope::try_new(client::io::ClientRevision::R289, "test-content").unwrap();
    let key = LoadoutKey::Account {
        account: account.clone(),
        scope: scope.clone(),
    };
    let identity = DomainIdentity::new(LocalStateDomain::LoadoutsReferences, account, scope);
    (key, identity)
}

fn read_loadouts(
    service: &LocalStateService,
    key: &LoadoutKey,
) -> Result<LoadoutSnapshot, ErrorCode> {
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
        let remaining = deadline.saturating_duration_since(Instant::now());
        match service.wait_loadout_readback(key, ticket, remaining) {
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
        other => panic!("loadout edit did not settle: {other:?}"),
    }
}

fn migrated(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap().to_os_string();
    name.push(".migrated");
    path.with_file_name(name)
}

fn head_counts(root: &StateRoot, store: &str) -> Option<(i64, i64)> {
    inspect(root)
        .query_row(
            "SELECT unreadable,lossy FROM loadout_head WHERE store=?1",
            params![store],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .unwrap()
}

fn head_version(root: &StateRoot, store: &str) -> Option<u64> {
    let bytes: Option<Vec<u8>> = inspect(root)
        .query_row(
            "SELECT version FROM loadout_head WHERE store=?1",
            params![store],
            |row| row.get(0),
        )
        .optional()
        .unwrap();
    bytes.map(|bytes| sql::decode_u64(&bytes).unwrap())
}
fn head_digest(root: &StateRoot, store: &str) -> Option<Option<Vec<u8>>> {
    inspect(root)
        .query_row(
            "SELECT digest FROM loadout_head WHERE store=?1",
            params![store],
            |row| row.get(0),
        )
        .optional()
        .unwrap()
}

fn ledger(root: &StateRoot) -> Option<(String, String)> {
    inspect(root)
        .query_row(
            "SELECT kind,evidence FROM import_ledger WHERE domain='loadouts-references' AND source='loadouts.json'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .unwrap()
}

fn ledger_count(root: &StateRoot) -> i64 {
    inspect(root)
        .query_row(
            "SELECT count(*) FROM import_ledger WHERE domain='loadouts-references' AND source='loadouts.json'",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

fn stored_rows(root: &StateRoot, store: &str) -> Vec<(String, Value)> {
    let connection = inspect(root);
    let mut statement = connection
        .prepare("SELECT name,body FROM loadout_row WHERE store=?1 ORDER BY position")
        .unwrap();
    statement
        .query_map(params![store], |row| {
            let name: String = row.get(0)?;
            let body: Vec<u8> = row.get(1)?;
            Ok((name, body))
        })
        .unwrap()
        .map(|row| {
            let (name, body) = row.unwrap();
            (name, serde_json::from_slice(&body).unwrap())
        })
        .collect()
}

fn bank_omitted(root: &StateRoot) -> Option<i64> {
    inspect(root)
        .query_row(
            "SELECT omitted FROM bank_hint WHERE profile='local-289' AND account='alice' AND revision=289 AND content='test-content'",
            [],
            |row| row.get(0),
        )
        .optional()
        .unwrap()
}

#[test]
fn mixed_legacy_loadouts_import_losslessly_save_and_restart() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("s6a-loadouts-mixed");
    let root = root_of(&scratch);
    let source = root.loadouts_file();
    let bytes = br#"[
      {"name":"  legacy mixed  ","worn":[" old hat "," old blade "],"unassigned":[" spare one "," spare two "],"carry":[" tonic ",{"item":" arrows ","qty":7},{"item":" bandage "},{"item":" zero ","qty":0},{"item":" negative ","qty":-1},{"item":" fractional ","qty":3.75},{"item":" huge ","qty":4294967296}]},
      {"name":"slots","worn":{"hat":" helm ","righthand":" sword ","future_slot":" relic "},"carry":[{"item":" bolts ","qty":4}]},
      {"name":"current","worn":{"back":" cloak "},"carry":[{"item":" food ","qty":2}],"unassigned":[" spare "]}
    ]"#;
    write_private(&source, bytes);
    let connections = sql::connections_opened_for_test(&root);
    let key = LoadoutKey::OperatorPresets;
    let service = open(&scratch);
    let snapshot = read_loadouts(&service, &key).expect("legacy projection imports");
    let expected = vec![
        Loadout {
            name: "legacy mixed".into(),
            worn: BTreeMap::new(),
            unassigned: vec!["old hat", "old blade", "spare one", "spare two"]
                .into_iter().map(str::to_owned).collect(),
            carry: vec![
                CarryEntry::new("tonic", 1), CarryEntry::new("arrows", 7),
                CarryEntry::new("bandage", 1), CarryEntry::new("zero", 1),
                CarryEntry::new("negative", 1), CarryEntry::new("fractional", 3),
                CarryEntry::new("huge", u32::MAX),
            ],
        },
        Loadout {
            name: "slots".into(),
            worn: BTreeMap::from([("hat".into(), "helm".into()), ("righthand".into(), "sword".into())]),
            unassigned: vec!["relic".into()],
            carry: vec![CarryEntry::new("bolts", 4)],
        },
        Loadout {
            name: "current".into(),
            worn: BTreeMap::from([("back".into(), "cloak".into())]),
            unassigned: vec!["spare".into()],
            carry: vec![CarryEntry::new("food", 2)],
        },
    ];
    assert_eq!(snapshot.key(), &key);
    assert_eq!(snapshot.rows(), expected.as_slice());
    assert_eq!(snapshot.unreadable_rows(), 0);
    assert_eq!(snapshot.lossy_rows(), 0);
    assert!(!snapshot.read_only());
    assert_eq!(head_counts(&root, OPERATOR_STORE), Some((0, 0)));
    assert_eq!(ledger(&root), Some(("imported".into(), "completed".into())));
    assert!(!source.exists());
    assert_eq!(std::fs::read(migrated(&source)).unwrap(), bytes);
    let db_rows = stored_rows(&root, OPERATOR_STORE);
    assert_eq!(db_rows.len(), 3);
    assert_eq!(db_rows[0].0, "legacy mixed");
    assert!(db_rows[0].1.get("name").is_none());
    assert_eq!(db_rows[0].1["unassigned"], json!(&expected[0].unassigned));
    assert_eq!(db_rows[0].1["carry"], serde_json::to_value(&expected[0].carry).unwrap());

    let mut replacement = expected[0].clone();
    replacement.carry[0] = CarryEntry::new("tonic", 2);
    let edit = LoadoutEdit::upsert(operator_identity(), Some(snapshot.version()), replacement.clone()).unwrap();
    save_loadout(&service, edit).expect("supported conversion remains writable");
    let saved = service.loadout_snapshot(&key).unwrap();
    let mut want = expected;
    want[0] = replacement;
    assert_eq!(saved.rows(), want.as_slice());
    drop(service);

    let reopened = open(&scratch);
    let after_restart = read_loadouts(&reopened, &key).unwrap();
    assert_eq!(after_restart.rows(), want.as_slice());
    assert_eq!(after_restart.version(), saved.version());
    assert_eq!(sql::connections_opened_for_test(&root), connections + 2);
    assert_eq!(std::fs::read(migrated(&source)).unwrap(), bytes);
    assert!(!source.exists());
}

#[test]
fn duplicate_members_and_names_refuse_the_whole_loadout_source() {
    let _state_lease = crate::test_process::state();
    for (label, bytes) in [
        ("nested-members", &br#"[{"name":"one","worn":{"hat":"cap","hat":"helm"},"carry":[]}]"#[..]),
        ("duplicate-names", &br#"[{"name":"same","worn":{},"carry":[]},{"name":"same","worn":{"hat":"cap"},"carry":[]}]"#[..]),
    ] {
        let scratch = IsolatedEnv::enter(&format!("s6a-loadouts-{label}"));
        let root = root_of(&scratch);
        let source = root.loadouts_file();
        write_private(&source, bytes);
        let service = open(&scratch);
        let error = read_loadouts(&service, &LoadoutKey::OperatorPresets).unwrap_err();
        assert!(!error.is_not_ready(), "source parsing never ran: {error:?}");
        assert_eq!(head_counts(&root, OPERATOR_STORE), None);
        assert_eq!(ledger_count(&root), 0);
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
        assert!(!migrated(&source).exists());
    }
}

#[test]
fn operator_and_account_loadout_keys_keep_independent_versions_and_rows() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("s6a-loadouts-keyed");
    let root = root_of(&scratch);
    let service = open(&scratch);
    let operator_key = LoadoutKey::OperatorPresets;
    let (account_key, account_identity) = account_context("alice");
    let operator_empty = read_loadouts(&service, &operator_key).unwrap();
    let account_empty = read_loadouts(&service, &account_key).unwrap();
    assert!(operator_empty.rows().is_empty());
    assert!(account_empty.rows().is_empty());
    assert!(service.readback_lease(&operator_identity(), LOADOUT_WAIT).unwrap().is_none());
    assert_eq!(head_counts(&root, OPERATOR_STORE), Some((0, 0)));
    assert_eq!(head_digest(&root, OPERATOR_STORE), Some(None));
    assert_eq!(head_version(&root, OPERATOR_STORE), Some(operator_empty.version().get()));
    assert_eq!(ledger(&root), Some(("absent".into(), "completed".into())));

    let operator_row = Loadout::new("operator-kit").with_slot("hat", "helm");
    save_loadout(&service, LoadoutEdit::upsert(operator_identity(), None, operator_row.clone()).unwrap()).unwrap();
    let operator_v1 = service.loadout_snapshot(&operator_key).unwrap();
    let account_row = Loadout::new("account-kit").with_carry("food", 3);
    let account_edit = LoadoutEdit::upsert(account_identity.clone(), None, account_row.clone())
        .unwrap().with_key(account_key.clone()).unwrap();
    save_loadout(&service, account_edit).unwrap();
    let account_v1 = service.loadout_snapshot(&account_key).unwrap();
    assert_eq!(operator_v1.key(), &operator_key);
    assert_eq!(account_v1.key(), &account_key);
    assert_eq!(operator_v1.rows(), std::slice::from_ref(&operator_row));
    assert_eq!(account_v1.rows(), std::slice::from_ref(&account_row));

    let (_, bob_identity) = account_context("bob");
    let mismatch = LoadoutEdit::upsert(bob_identity, None, Loadout::new("wrong"))
        .unwrap().with_key(account_key.clone());
    assert!(mismatch.is_err(), "a key cannot select another account's identity");

    let mut account_row_v2 = account_row.clone();
    account_row_v2.carry[0].qty = 4;
    let account_edit = LoadoutEdit::upsert(account_identity, Some(account_v1.version()), account_row_v2.clone())
        .unwrap().with_key(account_key.clone()).unwrap();
    save_loadout(&service, account_edit).unwrap();
    let account_v2 = service.loadout_snapshot(&account_key).unwrap();
    let operator_row_v2 = Loadout::new("operator-kit").with_slot("hat", "crown");
    save_loadout(&service, LoadoutEdit::upsert(operator_identity(), Some(operator_v1.version()), operator_row_v2.clone()).unwrap()).unwrap();
    let operator_v2 = service.loadout_snapshot(&operator_key).unwrap();
    let stale = LoadoutEdit::upsert(operator_identity(), Some(operator_v1.version()), Loadout::new("stale")).unwrap();
    assert!(matches!(save_loadout(&service, stale), Err(ErrorCode::Conflict { .. })));
    assert_eq!(service.loadout_snapshot(&operator_key).unwrap().rows(), std::slice::from_ref(&operator_row_v2));
    assert_eq!(service.loadout_snapshot(&account_key).unwrap().rows(), std::slice::from_ref(&account_row_v2));
    assert!(operator_v2.version() > operator_v1.version());
    assert!(account_v2.version() > account_v1.version());
    assert_eq!(head_version(&root, OPERATOR_STORE), Some(operator_v2.version().get()));
    assert_eq!(head_version(&root, "account/local-289/alice/289/test-content"), Some(account_v2.version().get()));
}

#[test]
fn loadout_projection_acceptance_persists_counts_and_does_not_accept_bank_projection() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("s6a-loadouts-projection");
    let root = root_of(&scratch);
    let source = root.loadouts_file();
    let bytes = br#"[{"name":"usable","worn":["cape"],"carry":"food"},{"name":"lossy","worn":{},"carry":[],"future":{"nested":[1]}},{"worn":{"hat":"cap"},"carry":[]}]"#;
    write_private(&source, bytes);
    let bank_bytes = serde_json::to_vec(&json!({
        "schema_version":1,"profile":"local-289","account":"alice","revision":289,
        "content":"test-content","observed_at_unix":11,"rows":[[995,11]],"future":{"nested":[1]}
    })).unwrap();
    write_private(&root.bank_hint_file("local-289", "alice"), &bank_bytes);
    let (account_key, account_identity) = account_context("alice");
    let service = open(&scratch);
    let operator_key = LoadoutKey::OperatorPresets;
    let projected = read_loadouts(&service, &operator_key).unwrap();
    assert_eq!(projected.rows().len(), 2);
    assert_eq!(projected.unreadable_rows(), 1);
    assert_eq!(projected.lossy_rows(), 1);
    assert!(projected.read_only());
    let bank = service.readback_lease(&bank_identity("alice"), LOADOUT_WAIT).unwrap().unwrap();
    assert_eq!(bank.snapshot().as_bank_hints().unwrap(), &[(995, 11)]);
    assert_eq!(bank_omitted(&root), Some(1));

    let account_row = Loadout::new("account-only").with_carry("supply", 2);
    let account_edit = LoadoutEdit::upsert(account_identity, None, account_row.clone())
        .unwrap().with_key(account_key.clone()).unwrap();
    save_loadout(&service, account_edit).unwrap();
    let account_before = service.loadout_snapshot(&account_key).unwrap();
    let refused = save_loadout(
        &service,
        LoadoutEdit::upsert(operator_identity(), Some(projected.version()), Loadout::new("blocked")).unwrap(),
    );
    assert!(refused.is_err(), "lossy operator projection stays read-only");
    assert_eq!(service.loadout_snapshot(&account_key).unwrap().rows(), std::slice::from_ref(&account_row));
    drop(service);

    let reopened = open(&scratch);
    let persisted = read_loadouts(&reopened, &operator_key).unwrap();
    assert_eq!((persisted.unreadable_rows(), persisted.lossy_rows()), (1, 1));
    assert!(persisted.read_only());
    let bank = reopened.readback_lease(&bank_identity("alice"), LOADOUT_WAIT).unwrap().unwrap();
    assert_eq!(bank.snapshot().as_bank_hints().unwrap(), &[(995, 11)]);
    let account_after = read_loadouts(&reopened, &account_key).unwrap();
    assert_eq!(account_after.version(), account_before.version());
    assert_eq!(account_after.rows(), std::slice::from_ref(&account_row));
    drop(reopened);

    let questions = Mutex::new(Vec::<String>::new());
    let declined = run_maintenance(
        &root, Frontend::Panel,
        &MaintenanceAction::AcceptLoadoutProjection(OPERATOR_STORE.into()),
        &|question| { questions.lock().unwrap().push(question.to_owned()); false },
    ).unwrap_err();
    assert!(declined.message().contains("Not confirmed"));
    let question = questions.lock().unwrap().pop().unwrap();
    for required in [OPERATOR_STORE, "1 unreadable", "1 lossy", "loadouts.json.migrated"] {
        assert!(question.contains(required), "confirmation omits {required:?}: {question}");
    }
    assert!(question.contains(&persisted.version().get().to_string()));
    assert_eq!(head_counts(&root, OPERATOR_STORE), Some((1, 1)));

    let evidence = migrated(&source);
    let held = scratch.home.join("loadouts-evidence-held");
    std::fs::rename(&evidence, &held).unwrap();
    let missing = run_maintenance(
        &root, Frontend::Panel,
        &MaintenanceAction::AcceptLoadoutProjection(OPERATOR_STORE.into()), &|_| true,
    ).unwrap_err();
    assert!(missing.message().contains("evidence") || missing.message().contains("kept"));
    assert_eq!(head_counts(&root, OPERATOR_STORE), Some((1, 1)));
    std::fs::rename(&held, &evidence).unwrap();

    let stale = run_maintenance(
        &root, Frontend::Panel,
        &MaintenanceAction::AcceptLoadoutProjection(OPERATOR_STORE.into()),
        &|_| {
            let bytes = sql::encode_u64(persisted.version().get() + 1);
            inspect(&root).execute(
                "UPDATE loadout_head SET version=?1 WHERE store=?2",
                params![bytes.as_ref(), OPERATOR_STORE],
            ).unwrap();
            true
        },
    ).unwrap_err();
    assert!(stale.message().contains("version") || stale.message().contains("changed"));
    assert_eq!(head_counts(&root, OPERATOR_STORE), Some((1, 1)));
    run_maintenance(
        &root, Frontend::Panel,
        &MaintenanceAction::AcceptLoadoutProjection(OPERATOR_STORE.into()), &|_| true,
    ).expect("confirmed current projection is accepted");
    assert_eq!(head_counts(&root, OPERATOR_STORE), Some((0, 0)));
    assert_eq!(std::fs::read(&evidence).unwrap(), bytes);

    let accepted = open(&scratch);
    let operator_after = read_loadouts(&accepted, &operator_key).unwrap();
    assert!(!operator_after.read_only());
    assert_eq!(bank_omitted(&root), Some(1));
    let bank = accepted.readback_lease(&bank_identity("alice"), LOADOUT_WAIT).unwrap().unwrap();
    assert_eq!(bank.snapshot().as_bank_hints().unwrap(), &[(995, 11)]);
    assert!(matches!(accepted.submit(bank_hint("alice", 12)), SubmitReceipt::Refused(_)));
    let account_after = read_loadouts(&accepted, &account_key).unwrap();
    assert_eq!(account_after.version(), account_before.version());
    assert_eq!(account_after.rows(), std::slice::from_ref(&account_row));
}

#[test]
#[ignore = "child process exits after the loadout import commit"]
fn loadout_import_evidence_crash_child() {
    let root = StateRoot::new(env_path("S6_LOADOUT_ROOT"));
    let point: Arc<dyn Fn(&'static str) + Send + Sync> = Arc::new(|name| {
        if name == "before-evidence" { std::process::exit(79); }
    });
    let service = LocalStateService::open_process_with_seams_for_test(
        root, sql::Seams { point: Some(point), ..sql::Seams::default() }, None,
    );
    let key = LoadoutKey::OperatorPresets;
    let ticket = service.start_loadout_readback(&key).unwrap();
    let _ = service.wait_loadout_readback(&key, ticket, LOADOUT_WAIT);
}

#[test]
fn interrupted_loadout_import_retries_pending_evidence_once() {
    crate::test_process::isolate_state_test!(interrupted_loadout_import_retries_pending_evidence_once);
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("s6a-loadouts-evidence-retry");
    let root = root_of(&scratch);
    let source = root.loadouts_file();
    let bytes = br#"[{"name":"recovered","worn":["cape"],"carry":"food"}]"#;
    write_private(&source, bytes);
    let status = std::process::Command::new(crate::test_process::executable())
        .args(["--ignored", "--exact", "local_state::sql_tests::loadouts_tests::loadout_import_evidence_crash_child"])
        .env("S6_LOADOUT_ROOT", root.path())
        .status().unwrap();
    assert_eq!(status.code(), Some(79));
    assert_eq!(ledger(&root), Some(("pending".into(), "loadouts.json.migrated".into())));
    let committed = head_version(&root, OPERATOR_STORE).unwrap();
    assert!(source.exists());
    assert_eq!(sql::connections_opened_for_test(&root), 0);

    let service = open(&scratch);
    let snapshot = read_loadouts(&service, &LoadoutKey::OperatorPresets).unwrap();
    assert_eq!(snapshot.version().get(), committed);
    assert_eq!(snapshot.rows().len(), 1);
    assert_eq!(snapshot.rows()[0].name, "recovered");
    assert_eq!(ledger(&root), Some(("imported".into(), "completed".into())));
    assert!(!source.exists());
    assert_eq!(std::fs::read(migrated(&source)).unwrap(), bytes);
    assert_eq!(sql::connections_opened_for_test(&root), 1);
}
