use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use api::identity::{Cancellation, DurableTimestamp, RequestId, Settlement};
use api::loadout::Loadout;
use api::local_state::{
    AccountKey, AuditCommit, AuditOrigin, BankHint, CommitAction, ContentScope, DomainEdit,
    DomainIdentity, ErrorCode, FailureReason, LoadoutEdit, LocalStateDomain, PreferencesEdit,
    StoreVersion, SubmitReceipt,
};
use script::IsolatedEnv;

const PROFILE: &str = "local-289";
const CONTENT: &str = "test-content";
const WAIT: Duration = Duration::from_secs(10);

fn service(scratch: &IsolatedEnv, domains: &[LocalStateDomain]) -> Arc<LocalStateService> {
    LocalStateService::open(
        StateRoot::new(scratch.home.join(".274bot")),
        domains,
        Arc::new(RequestIdMinter::new()),
    )
}

fn identity(domain: LocalStateDomain, account: &str) -> DomainIdentity {
    DomainIdentity::new(
        domain,
        AccountKey::try_new(PROFILE, account).unwrap(),
        ContentScope::try_new(client::io::ClientRevision::R289, CONTENT).unwrap(),
    )
}

fn bank_identity(account: &str) -> DomainIdentity {
    identity(LocalStateDomain::BankHints, account)
}

fn loadouts_identity() -> DomainIdentity {
    identity(LocalStateDomain::LoadoutsReferences, "alice")
}

fn request(receipt: SubmitReceipt) -> RequestId {
    match receipt {
        SubmitReceipt::Accepted { request } => request,
        SubmitReceipt::Refused(code) => panic!("submission refused: {code}"),
    }
}

fn settlement(service: &LocalStateService, request: RequestId) -> WriteSettlement {
    service
        .wait_settled(request, WAIT)
        .unwrap_or_else(|| panic!("request {} did not settle", request.get()))
}

fn hint_edit(
    identity: &DomainIdentity,
    expected: Option<StoreVersion>,
    rows: Vec<(i32, i32)>,
    observed_at: u64,
) -> DomainEdit {
    DomainEdit::Bank(
        BankHint::try_new(
            identity.clone(),
            expected,
            rows,
            DurableTimestamp::from_unix_seconds(observed_at),
        )
        .unwrap(),
    )
}

fn hint_document(
    profile: &str,
    account: &str,
    schema_version: u32,
    observed_at_unix: u64,
    rows: &[(i32, i32)],
) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": schema_version,
        "profile": profile,
        "account": account,
        "observed_at_unix": observed_at_unix,
        "rows": rows,
    }))
    .unwrap()
}

fn write_private(path: &std::path::Path, bytes: &[u8]) {
    vault::write_private_file(path, bytes).unwrap();
}

#[test]
fn cold_lease_hydrates_to_an_empty_initial_store_and_never_written_is_none() {
    let scratch = IsolatedEnv::enter("local-state-cold");
    let service = service(&scratch, &[LocalStateDomain::BankHints]);
    let identity = bank_identity("alice");

    assert!(matches!(
        service.lease(&identity),
        Err(ErrorCode::NotReady {
            domain: LocalStateDomain::BankHints
        })
    ));
    assert_eq!(
        service.readback_lease(&identity, WAIT).unwrap(),
        None,
        "a store that has never been written has no file lease"
    );
    let lease = service.lease(&identity).unwrap();
    assert_eq!(lease.version(), StoreVersion::INITIAL);
    assert!(
        lease.snapshot().as_bank_hints().unwrap().is_empty(),
        "after readback the missing file is a real empty lease"
    );
}

#[test]
fn loadouts_store_and_service_exchange_identical_rows_and_pretty_json() {
    let scratch = IsolatedEnv::enter("local-state-loadouts");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.loadouts_file();
    let first = Loadout::new("starter")
        .with_slot("righthand", "Bronze sword")
        .with_carry("Shrimps", 4);
    let mut operator_store = script::LoadoutsStore::at(path.clone());
    operator_store.upsert(first.clone());
    operator_store.save().unwrap();

    let service = LocalStateService::open(
        root,
        &[LocalStateDomain::LoadoutsReferences],
        Arc::new(RequestIdMinter::new()),
    );
    let identity = loadouts_identity();
    let lease = service.readback_lease(&identity, WAIT).unwrap().unwrap();
    assert_eq!(lease.snapshot().as_loadouts().unwrap(), &[first.clone()]);

    let second = Loadout::new("banking").with_carry("Coins", 250);
    let request = request(service.submit(DomainEdit::Loadout(
        LoadoutEdit::upsert(identity, Some(lease.version()), second.clone()).unwrap(),
    )));
    let receipt = match settlement(&service, request) {
        Settlement::Completed(receipt) => receipt,
        other => panic!("loadout edit did not complete: {other:?}"),
    };
    let rows = vec![first, second];
    let expected = serde_json::to_string_pretty(&rows).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), expected.as_bytes());
    let operator_store = script::LoadoutsStore::at(path);
    assert_eq!(operator_store.loadouts(), rows.as_slice());
    assert_eq!(receipt.version(), lease.version().checked_next().unwrap());

    // The receipt is the durable source for the audit version and timestamp.
    // AuditCommit is deliberately not read back from log text.
    let audit = AuditCommit::after_commit(
        receipt.identity().clone(),
        CommitAction::LoadoutWrite,
        lease.version(),
        receipt.version(),
        AuditOrigin::Submitted { request },
        receipt.at(),
    );
    assert_eq!(audit.action(), CommitAction::LoadoutWrite);
    assert_eq!(audit.previous(), lease.version());
    assert_eq!(audit.version(), receipt.version());
    assert_eq!(audit.identity(), receipt.identity());
    assert_eq!(audit.origin(), AuditOrigin::Submitted { request });
    assert_eq!(audit.at(), receipt.at());
}

#[test]
fn bank_hint_submission_writes_schema_one_and_readback_preserves_rows_and_time() {
    let scratch = IsolatedEnv::enter("local-state-bank-format");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    let alice = bank_identity("alice");
    let at = 1_759_700_123;
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    let request =
        request(service.submit(hint_edit(&alice, None, vec![(379, 12), (995, 3400)], at)));
    let receipt = match settlement(&service, request) {
        Settlement::Completed(receipt) => receipt,
        other => panic!("bank hint did not complete: {other:?}"),
    };
    let path = root.bank_hint_file(PROFILE, "alice");
    let written: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(written["schema_version"], serde_json::json!(1));
    assert_eq!(written["profile"], serde_json::json!(PROFILE));
    assert_eq!(written["account"], serde_json::json!("alice"));
    assert_eq!(written["observed_at_unix"], serde_json::json!(at));
    assert_eq!(written["rows"], serde_json::json!([[379, 12], [995, 3400]]));
    assert_eq!(
        receipt.version(),
        StoreVersion::INITIAL.checked_next().unwrap()
    );

    let bob = bank_identity("bob");
    let bob_at = 1_759_700_456;
    let bob_rows = [(526, 5), (995, 70)];
    let bob_path = root.bank_hint_file(PROFILE, "bob");
    write_private(
        &bob_path,
        &hint_document(PROFILE, "bob", HINT_SCHEMA_VERSION, bob_at, &bob_rows),
    );
    let lease = service.readback_lease(&bob, WAIT).unwrap().unwrap();
    assert_eq!(lease.snapshot().as_bank_hints().unwrap(), bob_rows);
    assert_eq!(lease.at().unix_seconds(), bob_at);
}

#[test]
fn expected_version_conflict_settles_without_touching_the_file() {
    let scratch = IsolatedEnv::enter("local-state-conflict");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::LoadoutsReferences],
        Arc::new(RequestIdMinter::new()),
    );
    let identity = loadouts_identity();
    let initial = service.readback_lease(&identity, WAIT).unwrap();
    assert!(initial.is_none());
    let stale = StoreVersion::INITIAL;
    let first = request(
        service.submit(DomainEdit::Loadout(
            LoadoutEdit::upsert(
                identity.clone(),
                None,
                Loadout::new("first").with_carry("Coins", 10),
            )
            .unwrap(),
        )),
    );
    let first_receipt = match settlement(&service, first) {
        Settlement::Completed(receipt) => receipt,
        other => panic!("initial write did not complete: {other:?}"),
    };
    let path = root.loadouts_file();
    let before = std::fs::read(&path).unwrap();
    let conflict = request(
        service.submit(DomainEdit::Loadout(
            LoadoutEdit::upsert(
                identity,
                Some(stale),
                Loadout::new("stale").with_carry("Coins", 99),
            )
            .unwrap(),
        )),
    );
    assert_eq!(
        settlement(&service, conflict),
        Settlement::Failed(ErrorCode::Conflict {
            expected: stale,
            found: first_receipt.version(),
        })
    );
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn a_new_queued_hint_supersedes_the_old_one_and_quiescence_tracks_the_newest() {
    let scratch = IsolatedEnv::enter("local-state-superseded");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[
            LocalStateDomain::BankHints,
            LocalStateDomain::LoadoutsReferences,
        ],
        Arc::new(RequestIdMinter::new()),
    );
    let bank = bank_identity("alice");
    let loadouts = loadouts_identity();
    let _ = service.readback_lease(&bank, WAIT).unwrap();
    let _ = service.readback_lease(&loadouts, WAIT).unwrap();
    let blocker_path = root.loadouts_file();
    let bank_path = root.bank_hint_file(PROFILE, "alice");
    let hook_bank_path = bank_path.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    let bank_writes = Arc::new(AtomicUsize::new(0));
    let counted_bank_writes = Arc::clone(&bank_writes);
    service.set_write_hook(Arc::new(move |path, bytes| {
        if path == blocker_path {
            entered_tx
                .send(())
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            let _ = release_rx
                .lock()
                .unwrap()
                .recv()
                .map_err(|error| std::io::Error::other(error.to_string()))?;
        }
        if path == hook_bank_path {
            counted_bank_writes.fetch_add(1, Ordering::SeqCst);
        }
        vault::write_private_file(path, bytes)
    }));

    let blocker = request(
        service.submit(DomainEdit::Loadout(
            LoadoutEdit::upsert(
                loadouts,
                None,
                Loadout::new("blocker").with_carry("Coins", 1),
            )
            .unwrap(),
        )),
    );
    entered_rx.recv_timeout(WAIT).unwrap();
    let first = request(service.submit(hint_edit(&bank, None, vec![(995, 1)], 100)));
    let second = request(service.submit(hint_edit(&bank, None, vec![(995, 2)], 101)));
    assert_eq!(
        service.observe(first),
        Some(Settlement::Cancelled(Cancellation::Superseded))
    );
    assert_eq!(service.observe(second), None, "the newest hint is queued");

    release_tx.send(()).unwrap();
    assert!(matches!(
        settlement(&service, blocker),
        Settlement::Completed(_)
    ));
    assert!(service.wait_quiescent(&bank, WAIT));
    assert!(matches!(
        settlement(&service, second),
        Settlement::Completed(_)
    ));
    assert_eq!(
        bank_writes.load(Ordering::SeqCst),
        1,
        "only the newest queued hint was written"
    );
    let written: serde_json::Value =
        serde_json::from_slice(&std::fs::read(bank_path).unwrap()).unwrap();
    assert_eq!(written["rows"], serde_json::json!([[995, 2]]));
    assert!(service.wait_quiescent(&bank, Duration::ZERO));
}

#[test]
fn identical_hint_bytes_complete_at_the_existing_version_without_a_write() {
    let scratch = IsolatedEnv::enter("local-state-identical");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    let identity = bank_identity("alice");
    assert_eq!(service.readback_lease(&identity, WAIT).unwrap(), None);
    let writes = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&writes);
    service.set_write_hook(Arc::new(move |path, bytes| {
        counted.fetch_add(1, Ordering::SeqCst);
        vault::write_private_file(path, bytes)
    }));
    let rows = vec![(995, 3400)];
    let at = 1_759_700_123;
    let first = request(service.submit(hint_edit(&identity, None, rows.clone(), at)));
    let first_receipt = match settlement(&service, first) {
        Settlement::Completed(receipt) => receipt,
        other => panic!("first hint did not complete: {other:?}"),
    };
    assert_eq!(writes.load(Ordering::SeqCst), 1);

    let second = request(service.submit(hint_edit(&identity, None, rows, at)));
    let second_receipt = match settlement(&service, second) {
        Settlement::Completed(receipt) => receipt,
        other => panic!("identical hint did not complete: {other:?}"),
    };
    assert_eq!(second_receipt.version(), first_receipt.version());
    assert_eq!(writes.load(Ordering::SeqCst), 1);
}

#[test]
fn failed_write_keeps_previous_rows_and_a_following_submission_can_succeed() {
    let scratch = IsolatedEnv::enter("local-state-write-failure");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    let identity = bank_identity("alice");
    assert_eq!(service.readback_lease(&identity, WAIT).unwrap(), None);
    let original = request(service.submit(hint_edit(&identity, None, vec![(995, 20)], 200)));
    assert!(matches!(
        settlement(&service, original),
        Settlement::Completed(_)
    ));
    let path = root.bank_hint_file(PROFILE, "alice");
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    service.set_write_hook(Arc::new(move |path, bytes| {
        if counted.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(std::io::Error::other("disk full"));
        }
        vault::write_private_file(path, bytes)
    }));

    let failed = request(service.submit(hint_edit(&identity, None, vec![(995, 25)], 201)));
    assert_eq!(
        settlement(&service, failed),
        Settlement::Failed(ErrorCode::Failed {
            domain: LocalStateDomain::BankHints,
            reason: FailureReason::Io,
        })
    );
    assert_eq!(
        service
            .lease(&identity)
            .unwrap()
            .snapshot()
            .as_bank_hints()
            .unwrap(),
        &[(995, 20)]
    );
    let before_retry = std::fs::read(&path).unwrap();
    let retry = request(service.submit(hint_edit(&identity, None, vec![(995, 25)], 201)));
    assert!(matches!(
        settlement(&service, retry),
        Settlement::Completed(_)
    ));
    assert_ne!(std::fs::read(path).unwrap(), before_retry);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn unopened_preferences_are_not_ready_for_leases_or_submissions() {
    let scratch = IsolatedEnv::enter("local-state-closed-domain");
    let service = service(&scratch, &[LocalStateDomain::BankHints]);
    let identity = identity(LocalStateDomain::Preferences, "alice");
    assert_eq!(
        service.lease(&identity),
        Err(ErrorCode::NotReady {
            domain: LocalStateDomain::Preferences
        })
    );
    let edit = PreferencesEdit::try_new(identity, None, "theme", "dark").unwrap();
    assert_eq!(
        service.submit(DomainEdit::Preferences(edit)),
        SubmitReceipt::Refused(ErrorCode::NotReady {
            domain: LocalStateDomain::Preferences
        })
    );
}

#[test]
fn external_loadout_edit_is_read_back_at_a_higher_version() {
    let scratch = IsolatedEnv::enter("local-state-external-edit");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.loadouts_file();
    let first = Loadout::new("first");
    let mut store = script::LoadoutsStore::at(path.clone());
    store.upsert(first.clone());
    store.save().unwrap();
    let service = LocalStateService::open(
        root,
        &[LocalStateDomain::LoadoutsReferences],
        Arc::new(RequestIdMinter::new()),
    );
    let identity = loadouts_identity();
    let before = service.readback_lease(&identity, WAIT).unwrap().unwrap();
    assert_eq!(before.snapshot().as_loadouts().unwrap(), &[first.clone()]);

    let second = Loadout::new("external");
    let mut store = script::LoadoutsStore::at(path);
    store.upsert(second.clone());
    store.save().unwrap();
    let after = service.readback_lease(&identity, WAIT).unwrap().unwrap();
    assert_eq!(after.snapshot().as_loadouts().unwrap(), &[first, second]);
    assert!(after.version() > before.version());
}

#[test]
fn wrong_hint_profile_account_or_schema_is_rejected_without_loading_memory() {
    let scratch = IsolatedEnv::enter("local-state-invalid-hints");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    let alice = bank_identity("alice");
    let bob = bank_identity("bob");
    let carol = bank_identity("carol");
    write_private(
        &root.bank_hint_file(PROFILE, "alice"),
        &hint_document(
            "another-profile",
            "alice",
            HINT_SCHEMA_VERSION,
            1,
            &[(995, 1)],
        ),
    );
    write_private(
        &root.bank_hint_file(PROFILE, "bob"),
        &hint_document(PROFILE, "alice", HINT_SCHEMA_VERSION, 2, &[(995, 2)]),
    );
    write_private(
        &root.bank_hint_file(PROFILE, "carol"),
        &hint_document(PROFILE, "carol", HINT_SCHEMA_VERSION + 1, 3, &[(995, 3)]),
    );
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    assert_eq!(service.readback_lease(&bob, WAIT).unwrap(), None);
    assert_eq!(service.readback_lease(&carol, WAIT).unwrap(), None);

    let memory = Arc::new(parking_lot::RwLock::new(
        api::bank_memory::BankMemory::default(),
    ));
    let port = crate::slot_bank_memory::SlotHintPort::new(Arc::clone(&service), carol);
    let slot = crate::slot_bank_memory::SlotBankMemory::new(Arc::clone(&memory), Some(port));
    slot.session_started();
    assert!(
        !memory.read().known(),
        "a refused hint leaves memory Unknown"
    );
}

#[test]
fn durable_receipts_are_exact_to_account_and_accounts_have_independent_versions() {
    let scratch = IsolatedEnv::enter("local-state-receipts");
    let service = service(&scratch, &[LocalStateDomain::BankHints]);
    let alice = bank_identity("alice");
    let bob = bank_identity("bob");
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    assert_eq!(service.readback_lease(&bob, WAIT).unwrap(), None);

    let alice_request = request(service.submit(hint_edit(&alice, None, vec![(995, 10)], 10)));
    let alice_receipt = match settlement(&service, alice_request) {
        Settlement::Completed(receipt) => receipt,
        other => panic!("alice's write did not complete: {other:?}"),
    };
    let bob_request = request(service.submit(hint_edit(&bob, None, vec![(995, 20)], 20)));
    let bob_receipt = match settlement(&service, bob_request) {
        Settlement::Completed(receipt) => receipt,
        other => panic!("bob's write did not complete: {other:?}"),
    };

    let first_version = StoreVersion::INITIAL.checked_next().unwrap();
    assert_eq!(alice_receipt.version(), first_version);
    assert_eq!(bob_receipt.version(), first_version);
    assert!(alice_receipt.accepts(&alice));
    assert!(!alice_receipt.accepts(&bob));
    assert!(bob_receipt.accepts(&bob));
    assert!(!bob_receipt.accepts(&alice));

    let audit = AuditCommit::after_commit(
        alice_receipt.identity().clone(),
        CommitAction::BankHintWrite,
        StoreVersion::INITIAL,
        alice_receipt.version(),
        AuditOrigin::Submitted {
            request: alice_request,
        },
        alice_receipt.at(),
    );
    assert_eq!(audit.previous(), StoreVersion::INITIAL);
    assert_eq!(audit.version(), alice_receipt.version());
    assert_eq!(audit.action(), CommitAction::BankHintWrite);
    assert_eq!(
        audit.origin(),
        AuditOrigin::Submitted {
            request: alice_request
        }
    );
    assert!(service.wait_quiescent(&alice, WAIT));
    assert!(service.wait_quiescent(&bob, WAIT));
}
