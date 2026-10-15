use super::*;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use api::identity::{Cancellation, DurableTimestamp, RequestId, Settlement};
use api::loadout::Loadout;
use api::local_state::{
    AccountKey, AuditCommit, AuditOrigin, BankHint, CommitAction, ContentScope, DomainEdit,
    DomainIdentity, ErrorCode, FailureReason, LoadoutEdit, LocalStateDomain, PreferencesEdit,
    StoreVersion, SubmitReceipt, ValidationReason,
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

fn scoped(domain: LocalStateDomain, account: &str, content: &str) -> DomainIdentity {
    DomainIdentity::new(
        domain,
        AccountKey::try_new(PROFILE, account).unwrap(),
        ContentScope::try_new(client::io::ClientRevision::R289, content).unwrap(),
    )
}

fn identity(domain: LocalStateDomain, account: &str) -> DomainIdentity {
    scoped(domain, account, CONTENT)
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

/// `request`'s settlement, waiting for it.
fn settlement(service: &LocalStateService, request: RequestId) -> WriteSettlement {
    settled_within(service, request, WAIT)
        .unwrap_or_else(|| panic!("request {} did not settle", request.get()))
}

/// `request`'s settlement if it lands within `wait`.
fn settled_within(
    service: &LocalStateService,
    request: RequestId,
    wait: Duration,
) -> Option<WriteSettlement> {
    match service.wait_request(request, wait) {
        RequestState::Settled(settlement) => Some(settlement),
        RequestState::Pending | RequestState::Forgotten => None,
    }
}

fn completed(service: &LocalStateService, request: RequestId) -> DurableReceipt {
    match settlement(service, request) {
        Settlement::Completed(receipt) => receipt,
        other => panic!("request {} did not complete: {other:?}", request.get()),
    }
}

/// How many stores the service holds.
fn held_stores(service: &LocalStateService) -> usize {
    service.shared.state.lock().stores.len()
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

fn loadout_edit(expected: Option<StoreVersion>, loadout: Loadout) -> DomainEdit {
    DomainEdit::Loadout(LoadoutEdit::upsert(loadouts_identity(), expected, loadout).unwrap())
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

/// A writer held inside the publication of `blocker` until released, so
/// later submissions queue behind it.
struct Stall {
    entered: mpsc::Receiver<()>,
    release: mpsc::Sender<()>,
}

fn stall_on(service: &LocalStateService, blocker: std::path::PathBuf) -> Stall {
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    service.set_write_hook(Arc::new(move |path, bytes| {
        if path == blocker {
            let _ = entered_tx.send(());
            let _ = release_rx.lock().unwrap().recv();
        }
        vault::write_private_file(path, bytes)
    }));
    Stall { entered, release }
}

/// Plain unsynced file writes and no directory flushes, for the tests that
/// count hundreds of settlements or stores (what they check is not
/// durability, and hundreds of real flushes slow every test beside them).
fn cheap_files(service: &LocalStateService) {
    service.set_write_hook(Arc::new(cheap_write));
    service.set_sync_hook(Arc::new(|_| Ok(())));
}

fn cheap_write(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(path.parent().expect("a state file has a directory"))?;
    std::fs::write(path, bytes)
}

/// Restores a directory's mode when the test ends, however it ends.
#[cfg(unix)]
struct ModeGuard(std::path::PathBuf);

#[cfg(unix)]
impl Drop for ModeGuard {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700));
    }
}

#[cfg(unix)]
fn running_as_root() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
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
    assert_eq!(
        lease.snapshot().as_loadouts().unwrap(),
        std::slice::from_ref(&first)
    );

    let second = Loadout::new("banking").with_carry("Coins", 250);
    let request = request(service.submit(loadout_edit(Some(lease.version()), second.clone())));
    let receipt = completed(&service, request);
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
    let receipt = completed(&service, request);
    let path = root.bank_hint_file(PROFILE, "alice");
    let written: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(written["schema_version"], serde_json::json!(1));
    assert_eq!(written["profile"], serde_json::json!(PROFILE));
    assert_eq!(written["account"], serde_json::json!("alice"));
    assert_eq!(written["revision"], serde_json::json!(289));
    assert_eq!(written["content"], serde_json::json!(CONTENT));
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
    let first = request(service.submit(loadout_edit(
        None,
        Loadout::new("first").with_carry("Coins", 10),
    )));
    let first_receipt = completed(&service, first);
    let path = root.loadouts_file();
    let before = std::fs::read(&path).unwrap();
    let conflict = request(service.submit(loadout_edit(
        Some(stale),
        Loadout::new("stale").with_carry("Coins", 99),
    )));
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
fn a_new_queued_hint_supersedes_the_old_one_which_follows_it_to_durability() {
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
            release_rx
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

    let blocker = request(service.submit(loadout_edit(
        None,
        Loadout::new("blocker").with_carry("Coins", 1),
    )));
    entered_rx.recv_timeout(WAIT).unwrap();
    let first = request(service.submit(hint_edit(&bank, None, vec![(995, 1)], 100)));
    let second = request(service.submit(hint_edit(&bank, None, vec![(995, 2)], 101)));
    assert_eq!(
        service.observe(first),
        Some(Settlement::Cancelled(Cancellation::Superseded))
    );
    assert_eq!(service.observe(second), None, "the newest hint is queued");
    assert_eq!(
        service.wait_durable(first, Duration::from_millis(50)),
        Durability::Pending,
        "a superseded hint is durable only when its replacement is"
    );

    release_tx.send(()).unwrap();
    assert!(matches!(
        settlement(&service, blocker),
        Settlement::Completed(_)
    ));
    let durable = service.wait_durable(first, WAIT);
    assert!(
        matches!(&durable, Durability::Durable(receipt) if receipt.request() == Some(second)),
        "{durable:?}"
    );
    assert_eq!(
        bank_writes.load(Ordering::SeqCst),
        1,
        "only the newest queued hint was written"
    );
    let written: serde_json::Value =
        serde_json::from_slice(&std::fs::read(bank_path).unwrap()).unwrap();
    assert_eq!(written["rows"], serde_json::json!([[995, 2]]));
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
    let first_receipt = completed(&service, first);
    assert_eq!(writes.load(Ordering::SeqCst), 1);

    let second = request(service.submit(hint_edit(&identity, None, rows, at)));
    let second_receipt = completed(&service, second);
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
    completed(&service, original);
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
    completed(&service, retry);
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
    assert_eq!(
        before.snapshot().as_loadouts().unwrap(),
        std::slice::from_ref(&first)
    );

    let second = Loadout::new("external");
    let mut store = script::LoadoutsStore::at(path);
    store.upsert(second.clone());
    store.save().unwrap();
    let after = service.readback_lease(&identity, WAIT).unwrap().unwrap();
    assert_eq!(after.snapshot().as_loadouts().unwrap(), &[first, second]);
    assert!(after.version() > before.version());
}

/// r2 item 5 (sol P1, grok High): a hint file another profile, account or
/// schema wrote is refused visibly, never read as "no file".
#[test]
fn wrong_hint_profile_account_or_schema_is_refused_not_absent_and_never_loaded() {
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
    let refused = |reason| ErrorCode::Failed {
        domain: LocalStateDomain::BankHints,
        reason,
    };
    assert_eq!(
        service.readback_lease(&alice, WAIT),
        Err(refused(FailureReason::Identity))
    );
    assert_eq!(
        service.readback_lease(&bob, WAIT),
        Err(refused(FailureReason::Identity))
    );
    assert_eq!(
        service.readback_lease(&carol, WAIT),
        Err(refused(FailureReason::Schema))
    );
    assert_eq!(service.lease(&carol), Err(refused(FailureReason::Schema)));

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

/// r2 item 5: every refused hint file the review attacked (unparseable,
/// another schema, another identity, group/other-writable, unreadable)
/// stays refused for leases and submissions and keeps its bytes.
#[cfg(unix)]
#[test]
fn refused_hint_files_are_never_overwritten() {
    if running_as_root() {
        return;
    }
    let scratch = IsolatedEnv::enter("local-state-refused-hints");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    let cases: [(&str, Vec<u8>, Option<u32>, FailureReason); 5] = [
        (
            "corrupt",
            b"{not json".to_vec(),
            None,
            FailureReason::Schema,
        ),
        (
            "schema",
            hint_document(PROFILE, "schema", 7, 1, &[(995, 1)]),
            None,
            FailureReason::Schema,
        ),
        (
            "identity",
            hint_document(PROFILE, "someone", HINT_SCHEMA_VERSION, 1, &[(995, 1)]),
            None,
            FailureReason::Identity,
        ),
        (
            "modesixsix",
            hint_document(PROFILE, "modesixsix", HINT_SCHEMA_VERSION, 1, &[(995, 1)]),
            Some(0o666),
            FailureReason::Io,
        ),
        (
            "modezero",
            hint_document(PROFILE, "modezero", HINT_SCHEMA_VERSION, 1, &[(995, 1)]),
            Some(0o000),
            FailureReason::Io,
        ),
    ];
    for (account, bytes, mode, reason) in cases {
        let identity = bank_identity(account);
        let path = root.bank_hint_file(PROFILE, account);
        write_private(&path, &bytes);
        if let Some(mode) = mode {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        let code = ErrorCode::Failed {
            domain: LocalStateDomain::BankHints,
            reason,
        };
        assert_eq!(
            service.readback_lease(&identity, WAIT),
            Err(code),
            "{account}"
        );
        assert_eq!(
            service.submit(hint_edit(&identity, None, vec![(995, 99)], 9)),
            SubmitReceipt::Refused(code),
            "{account}"
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "{account} kept");
    }
}

/// r2 item 5 (sol probe: a valid file past the read bound; grok: a
/// document that is not an array): refused loadouts are never replaced.
#[test]
fn refused_loadouts_files_are_never_overwritten() {
    let scratch = IsolatedEnv::enter("local-state-refused-loadouts");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.loadouts_file();
    let wrapped = serde_json::to_vec(
        &serde_json::json!({ "loadouts": [{"name": "kept", "worn": {}, "carry": []}] }),
    )
    .unwrap();
    let big_row = Loadout::new("big").with_carry("x".repeat(1024), 1);
    let oversize = serde_json::to_vec_pretty(&vec![big_row; 4200]).unwrap();
    assert!(oversize.len() as u64 > files::MAX_LOADOUTS_BYTES);
    for (label, bytes) in [
        ("wrapped", wrapped),
        ("oversize", oversize),
        ("corrupt", b"[{\"name\": ".to_vec()),
    ] {
        write_private(&path, &bytes);
        let service = LocalStateService::open(
            root.clone(),
            &[LocalStateDomain::LoadoutsReferences],
            Arc::new(RequestIdMinter::new()),
        );
        let refused = service.readback_lease(&loadouts_identity(), WAIT);
        assert!(
            matches!(
                refused,
                Err(ErrorCode::Failed {
                    domain: LocalStateDomain::LoadoutsReferences,
                    ..
                })
            ),
            "{label}: {refused:?}"
        );
        assert!(
            matches!(
                service.submit(loadout_edit(None, Loadout::new("new"))),
                SubmitReceipt::Refused(ErrorCode::Failed { .. })
            ),
            "{label}"
        );
        assert!(std::fs::read(&path).unwrap() == bytes, "{label} kept");
    }
}

/// r2 item 5: rows the service cannot read back as they are (a malformed
/// row, an unknown field) are leased, but the file is never rewritten
/// without them.
#[test]
fn a_loadouts_file_the_service_cannot_round_trip_is_read_but_never_rewritten() {
    let scratch = IsolatedEnv::enter("local-state-lossy-loadouts");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.loadouts_file();
    let bytes = serde_json::to_vec_pretty(&serde_json::json!([
        {"name": "melee", "worn": {"righthand": "Rune scimitar"}, "carry": [], "note": "keep me"},
        {"worn": {"hat": "no name"}},
    ]))
    .unwrap();
    write_private(&path, &bytes);
    let service = LocalStateService::open(
        root,
        &[LocalStateDomain::LoadoutsReferences],
        Arc::new(RequestIdMinter::new()),
    );
    let lease = service
        .readback_lease(&loadouts_identity(), WAIT)
        .unwrap()
        .unwrap();
    assert_eq!(
        lease.snapshot().as_loadouts().unwrap(),
        &[Loadout::new("melee").with_slot("righthand", "Rune scimitar")]
    );
    assert_eq!(
        service.submit(loadout_edit(Some(lease.version()), Loadout::new("ranged"))),
        SubmitReceipt::Refused(ErrorCode::Failed {
            domain: LocalStateDomain::LoadoutsReferences,
            reason: FailureReason::Schema,
        })
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

/// r2 item 5: a refused file is read again on demand; once repaired, it is
/// leased and written normally.
#[test]
fn a_repaired_file_is_accepted_again() {
    let scratch = IsolatedEnv::enter("local-state-repaired");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    let alice = bank_identity("alice");
    let path = root.bank_hint_file(PROFILE, "alice");
    write_private(&path, b"{oops");
    assert!(service.readback_lease(&alice, WAIT).is_err());
    std::fs::remove_file(&path).unwrap();
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    let written = request(service.submit(hint_edit(&alice, None, vec![(995, 5)], 5)));
    completed(&service, written);
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("[[995,5]]"));
}

/// r2 item 4 (sol probe 1): after another writer replaced the hint, an
/// identical resubmission is not a success over bytes it never wrote.
#[test]
fn a_hint_resubmitted_after_an_outside_write_conflicts_and_keeps_the_outside_bytes() {
    let scratch = IsolatedEnv::enter("local-state-outside-hint");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    let alice = bank_identity("alice");
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    let ten = hint_edit(&alice, None, vec![(995, 10)], 10);
    completed(&service, request(service.submit(ten.clone())));
    let path = root.bank_hint_file(PROFILE, "alice");
    let outside = hint_document(PROFILE, "alice", HINT_SCHEMA_VERSION, 20, &[(995, 20)]);
    write_private(&path, &outside);

    let again = request(service.submit(ten));
    assert!(
        matches!(
            settlement(&service, again),
            Settlement::Failed(ErrorCode::Conflict { .. })
        ),
        "the resubmission did not fabricate a write"
    );
    assert_eq!(std::fs::read(&path).unwrap(), outside);
    let lease = service.lease(&alice).unwrap();
    assert_eq!(lease.snapshot().as_bank_hints().unwrap(), &[(995, 20)]);
}

/// r2 item 4 (sol probe 2): a loadout edit written against the cached
/// version after the form saved another row conflicts; the saved row stays.
#[test]
fn a_loadout_edit_after_a_form_saved_a_row_conflicts_and_keeps_the_row() {
    let scratch = IsolatedEnv::enter("local-state-form-race");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.loadouts_file();
    let service = LocalStateService::open(
        root,
        &[LocalStateDomain::LoadoutsReferences],
        Arc::new(RequestIdMinter::new()),
    );
    let hydrated = service.readback_lease(&loadouts_identity(), WAIT).unwrap();
    assert!(hydrated.is_none());
    let version = service.lease(&loadouts_identity()).unwrap().version();
    let mut form = script::LoadoutsStore::at(path.clone());
    form.upsert(Loadout::new("frontend-saved"));
    form.save().unwrap();
    let saved = std::fs::read(&path).unwrap();

    let edit = request(service.submit(loadout_edit(Some(version), Loadout::new("service"))));
    assert!(matches!(
        settlement(&service, edit),
        Settlement::Failed(ErrorCode::Conflict { expected, .. }) if expected == version
    ));
    assert_eq!(std::fs::read(&path).unwrap(), saved);
    let lease = service.lease(&loadouts_identity()).unwrap();
    assert_eq!(
        lease.snapshot().as_loadouts().unwrap(),
        &[Loadout::new("frontend-saved")]
    );
}

/// r2 item 4: an edit accepted before another writer changed the file
/// conflicts instead of writing over the change, also when a readback saw
/// the change first (the precondition then matches the cache, but the edit
/// was accepted against what came before).
#[test]
fn an_edit_accepted_before_an_outside_change_conflicts() {
    let scratch = IsolatedEnv::enter("local-state-accepted-then-changed");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[
            LocalStateDomain::BankHints,
            LocalStateDomain::LoadoutsReferences,
        ],
        Arc::new(RequestIdMinter::new()),
    );
    let alice = bank_identity("alice");
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    let _ = service.readback_lease(&loadouts_identity(), WAIT).unwrap();
    let stall = stall_on(&service, root.loadouts_file());
    let blocker = request(service.submit(loadout_edit(None, Loadout::new("blocker"))));
    stall.entered.recv_timeout(WAIT).unwrap();
    let queued = request(service.submit(hint_edit(&alice, None, vec![(995, 1)], 1)));
    let path = root.bank_hint_file(PROFILE, "alice");
    let outside = hint_document(PROFILE, "alice", HINT_SCHEMA_VERSION, 2, &[(995, 2)]);
    write_private(&path, &outside);
    // A session boundary asks for the file while the writer is held: its
    // readback runs before the queued write and adopts the change.
    let reader = {
        let service = Arc::clone(&service);
        let alice = alice.clone();
        std::thread::spawn(move || service.readback_lease(&alice, WAIT))
    };
    let key = files::StoreKey::of(&alice).unwrap();
    while service.shared.state.lock().stores[&key].readback != Readback::Queued {
        std::thread::yield_now();
    }
    stall.release.send(()).unwrap();
    let seen = reader.join().unwrap().unwrap().unwrap();
    assert_eq!(seen.snapshot().as_bank_hints().unwrap(), &[(995, 2)]);
    completed(&service, blocker);
    assert!(matches!(
        settlement(&service, queued),
        Settlement::Failed(ErrorCode::Conflict { .. })
    ));
    assert_eq!(std::fs::read(&path).unwrap(), outside);
}

/// r2 item 6 (sol probe): rows observed under one content scope are never
/// leased, or overwritten, under another.
#[test]
fn a_hint_written_under_one_content_scope_is_refused_under_another() {
    let scratch = IsolatedEnv::enter("local-state-content-scope");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    let under_a = scoped(LocalStateDomain::BankHints, "alice", "content-a");
    let under_b = scoped(LocalStateDomain::BankHints, "alice", "content-b");
    assert_eq!(service.readback_lease(&under_a, WAIT).unwrap(), None);
    completed(
        &service,
        request(service.submit(hint_edit(&under_a, None, vec![(995, 7)], 7))),
    );
    let path = root.bank_hint_file(PROFILE, "alice");
    let written = std::fs::read(&path).unwrap();

    let identity_refusal = ErrorCode::Failed {
        domain: LocalStateDomain::BankHints,
        reason: FailureReason::Identity,
    };
    assert_eq!(
        service.readback_lease(&under_b, WAIT),
        Err(identity_refusal)
    );
    assert_eq!(service.lease(&under_b), Err(identity_refusal));
    assert_eq!(
        service.submit(hint_edit(&under_b, None, vec![(995, 8)], 8)),
        SubmitReceipt::Refused(identity_refusal)
    );
    assert_eq!(std::fs::read(&path).unwrap(), written);
    let lease = service.lease(&under_a).unwrap();
    assert_eq!(lease.identity(), &under_a);
    assert_eq!(lease.snapshot().as_bank_hints().unwrap(), &[(995, 7)]);
}

/// r2 item 6: a hint written before scopes existed is read under the scope
/// that asks, and the first write stamps it with that scope.
#[test]
fn a_hint_written_before_scopes_is_read_then_stamped() {
    let scratch = IsolatedEnv::enter("local-state-legacy-hint");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.bank_hint_file(PROFILE, "alice");
    write_private(
        &path,
        &hint_document(PROFILE, "alice", HINT_SCHEMA_VERSION, 3, &[(995, 3)]),
    );
    let service = LocalStateService::open(
        root,
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    let alice = bank_identity("alice");
    let lease = service.readback_lease(&alice, WAIT).unwrap().unwrap();
    assert_eq!(lease.snapshot().as_bank_hints().unwrap(), &[(995, 3)]);
    completed(
        &service,
        request(service.submit(hint_edit(&alice, None, vec![(995, 4)], 4))),
    );
    let written: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(written["revision"], serde_json::json!(289));
    assert_eq!(written["content"], serde_json::json!(CONTENT));
}

/// r2 item 1: a write whose directory entry cannot be flushed is not
/// acknowledged durable and is not audited. The root is made `-wx` so the
/// file can be staged and renamed but the directory cannot be opened to
/// flush it.
#[cfg(unix)]
#[test]
fn a_write_whose_directory_entry_cannot_be_flushed_is_not_durable() {
    if running_as_root() {
        return;
    }
    let scratch = IsolatedEnv::enter("local-state-dir-sync");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::LoadoutsReferences],
        Arc::new(RequestIdMinter::new()),
    );
    assert!(service
        .readback_lease(&loadouts_identity(), WAIT)
        .unwrap()
        .is_none());
    let audits = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&audits);
    service.set_audit_hook(Arc::new(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
    }));
    vault::create_private_dir(root.path()).unwrap();
    let guard = ModeGuard(root.path().to_path_buf());
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o300)).unwrap();

    let edit = loadout_edit(None, Loadout::new("melee"));
    let unsynced = request(service.submit(edit.clone()));
    assert_eq!(
        settlement(&service, unsynced),
        Settlement::Failed(ErrorCode::Failed {
            domain: LocalStateDomain::LoadoutsReferences,
            reason: FailureReason::Io,
        }),
        "a rename whose directory was not flushed is not durable"
    );
    assert_eq!(
        audits.load(Ordering::SeqCst),
        0,
        "no audit before durability"
    );

    drop(guard);
    let retried = request(service.submit(edit));
    completed(&service, retried);
    assert_eq!(
        audits.load(Ordering::SeqCst),
        1,
        "the durable retry is audited"
    );
}

/// r2 item 1: nothing acknowledges a write until its directory entries
/// are flushed: the receipt and the audit line both follow the flush, and
/// a file created in new directories flushes every entry up to the root's
/// parent.
#[test]
fn the_receipt_and_audit_follow_the_directory_flush() {
    let scratch = IsolatedEnv::enter("local-state-flush-order");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    let alice = bank_identity("alice");
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    let events = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    let synced = Arc::clone(&events);
    service.set_sync_hook(Arc::new(move |dir| {
        if synced.lock().unwrap().is_empty() {
            let _ = entered_tx.send(());
            let _ = release_rx.lock().unwrap().recv();
        }
        synced.lock().unwrap().push(dir.display().to_string());
        files::sync_directory(dir)
    }));
    let audited = Arc::clone(&events);
    service.set_audit_hook(Arc::new(move |audit| {
        audited
            .lock()
            .unwrap()
            .push(format!("audit {}", audit.version()));
    }));

    let write = request(service.submit(hint_edit(&alice, None, vec![(995, 1)], 1)));
    entered.recv_timeout(WAIT).unwrap();
    assert_eq!(
        service.request_state(write),
        RequestState::Pending,
        "renamed but not flushed: not acknowledged"
    );
    release.send(()).unwrap();
    let receipt = completed(&service, write);
    let hint_dir = root.path().join("bank-hints").join(PROFILE);
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            hint_dir.display().to_string(),
            root.path().join("bank-hints").display().to_string(),
            root.path().display().to_string(),
            scratch.home.display().to_string(),
            format!("audit {}", receipt.version()),
        ]
    );

    // A later write of the same file flushes its own directory only.
    events.lock().unwrap().clear();
    let second = request(service.submit(hint_edit(&alice, None, vec![(995, 2)], 2)));
    entered.recv_timeout(WAIT).unwrap();
    release.send(()).unwrap();
    let receipt = completed(&service, second);
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            hint_dir.display().to_string(),
            format!("audit {}", receipt.version()),
        ]
    );
}

/// r2 item 2 (sol P1, grok probe): dropping the last service handle
/// finishes every accepted write, queued or in flight, and joins the
/// writer before it returns.
#[test]
fn dropping_the_service_finishes_every_accepted_write_and_joins_the_writer() {
    let scratch = IsolatedEnv::enter("local-state-drain");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[
            LocalStateDomain::BankHints,
            LocalStateDomain::LoadoutsReferences,
        ],
        Arc::new(RequestIdMinter::new()),
    );
    let alice = bank_identity("alice");
    let bob = bank_identity("bob");
    let _ = service.readback_lease(&loadouts_identity(), WAIT).unwrap();
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    assert_eq!(service.readback_lease(&bob, WAIT).unwrap(), None);
    let stall = stall_on(&service, root.loadouts_file());
    let _blocker = request(service.submit(loadout_edit(None, Loadout::new("blocker"))));
    stall.entered.recv_timeout(WAIT).unwrap();
    let _alice = request(service.submit(hint_edit(&alice, None, vec![(995, 64)], 64)));
    let _bob = request(service.submit(hint_edit(&bob, None, vec![(995, 65)], 65)));

    let shared = Arc::clone(&service.shared);
    let release = stall.release;
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        let _ = release.send(());
    });
    drop(service);
    releaser.join().unwrap();

    assert_eq!(shared.state.lock().writer, WriterPhase::Exited);
    assert!(shared.writer.lock().is_none(), "the writer was joined");
    assert!(std::fs::read_to_string(root.loadouts_file())
        .unwrap()
        .contains("blocker"));
    for (account, coins) in [("alice", 64), ("bob", 65)] {
        let raw = std::fs::read_to_string(root.bank_hint_file(PROFILE, account))
            .unwrap_or_else(|error| panic!("{account}'s queued hint was dropped: {error}"));
        assert!(raw.contains(&format!("[[995,{coins}]]")), "{raw}");
    }
}

/// r2 item 2: a write that panics settles its request (and those queued
/// behind it) instead of leaving them pending, and the writer keeps
/// serving.
#[test]
fn a_panicking_write_fails_its_request_and_the_writer_keeps_serving() {
    let scratch = IsolatedEnv::enter("local-state-panic");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    let alice = bank_identity("alice");
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    let panicked = Arc::new(AtomicBool::new(false));
    let once = Arc::clone(&panicked);
    service.set_write_hook(Arc::new(move |path, bytes| {
        if !once.swap(true, Ordering::SeqCst) {
            panic!("injected writer panic");
        }
        vault::write_private_file(path, bytes)
    }));
    let doomed = request(service.submit(hint_edit(&alice, None, vec![(995, 1)], 1)));
    assert_eq!(
        settlement(&service, doomed),
        Settlement::Failed(ErrorCode::Failed {
            domain: LocalStateDomain::BankHints,
            reason: FailureReason::Dirty,
        })
    );
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    let next = request(service.submit(hint_edit(&alice, None, vec![(995, 2)], 2)));
    completed(&service, next);
    assert!(
        std::fs::read_to_string(root.bank_hint_file(PROFILE, "alice"))
            .unwrap()
            .contains("[[995,2]]")
    );
}

/// r2 item 7 (sol P1): bank identities are capped; a new one past the cap
/// evicts the least recently used idle store, which reads its file again
/// when it is next asked for.
#[test]
fn bank_identities_are_capped_by_evicting_idle_stores() {
    let scratch = IsolatedEnv::enter("local-state-identity-cap");
    let service = service(&scratch, &[LocalStateDomain::BankHints]);
    let names: Vec<String> = (0..MAX_BANK_STORES + 44)
        .map(|n| format!("acct{n}"))
        .collect();
    for name in &names {
        assert_eq!(service.readback_lease(&bank_identity(name), WAIT), Ok(None));
    }
    assert!(
        held_stores(&service) <= MAX_BANK_STORES,
        "{} stores held",
        held_stores(&service)
    );
    let first = bank_identity(&names[0]);
    assert!(
        matches!(service.lease(&first), Err(ErrorCode::NotReady { .. })),
        "the evicted store is cold again"
    );
    assert_eq!(service.readback_lease(&first, WAIT), Ok(None));
    assert!(held_stores(&service) <= MAX_BANK_STORES);
}

/// r2 item 7: with every held store busy, a new identity is refused
/// `Busy`, visibly, instead of growing the cache.
#[test]
fn with_every_held_store_busy_a_new_identity_is_refused() {
    let scratch = IsolatedEnv::enter("local-state-identity-busy");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    let blocker = bank_identity("blocker");
    let others: Vec<DomainIdentity> = (1..MAX_BANK_STORES)
        .map(|n| bank_identity(&format!("acct{n}")))
        .collect();
    // Every store is read before the writer is held (a held writer reads
    // nothing).
    assert_eq!(service.readback_lease(&blocker, WAIT), Ok(None));
    for identity in &others {
        assert_eq!(service.readback_lease(identity, WAIT), Ok(None));
    }
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    let blocker_path = root.bank_hint_file(PROFILE, "blocker");
    service.set_sync_hook(Arc::new(|_| Ok(())));
    service.set_write_hook(Arc::new(move |path, bytes| {
        if path == blocker_path {
            let _ = entered_tx.send(());
            let _ = release_rx.lock().unwrap().recv();
        }
        cheap_write(path, bytes)
    }));
    let held = request(service.submit(hint_edit(&blocker, None, vec![(995, 1)], 1)));
    entered.recv_timeout(WAIT).unwrap();
    for identity in &others {
        request(service.submit(hint_edit(identity, None, vec![(995, 1)], 1)));
    }
    let late = bank_identity("late");
    assert_eq!(
        service.lease(&late),
        Err(ErrorCode::Busy {
            domain: LocalStateDomain::BankHints
        })
    );
    assert_eq!(held_stores(&service), MAX_BANK_STORES);
    release.send(()).unwrap();
    completed(&service, held);
}

/// r2 item 7: an edit that would take the loadouts file past its row,
/// row-size or file-size budget is refused before it is accepted.
#[test]
fn loadout_edits_past_the_row_or_byte_budget_are_refused() {
    let scratch = IsolatedEnv::enter("local-state-loadout-budget");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.loadouts_file();
    let mut form = script::LoadoutsStore::at(path.clone());
    for n in 0..files::MAX_LOADOUT_ROWS {
        form.upsert(Loadout::new(format!("row{n}")));
    }
    form.save().unwrap();
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::LoadoutsReferences],
        Arc::new(RequestIdMinter::new()),
    );
    let version = service
        .readback_lease(&loadouts_identity(), WAIT)
        .unwrap()
        .unwrap()
        .version();
    assert_eq!(
        service.submit(loadout_edit(Some(version), Loadout::new("one-too-many"))),
        SubmitReceipt::Refused(ErrorCode::Validation {
            domain: LocalStateDomain::LoadoutsReferences,
            reason: ValidationReason::TooManyRows,
        })
    );
    let replaced = request(service.submit(loadout_edit(
        Some(version),
        Loadout::new("row0").with_carry("Coins", 1),
    )));
    completed(&service, replaced);

    let huge = Loadout::new("huge").with_carry("y".repeat(files::MAX_LOADOUT_ROW_BYTES), 1);
    assert_eq!(
        service.submit(loadout_edit(None, huge)),
        SubmitReceipt::Refused(ErrorCode::Validation {
            domain: LocalStateDomain::LoadoutsReferences,
            reason: ValidationReason::TooLong,
        })
    );

    // At the file bound: as many rows as fit, then one more is refused.
    let wide = |name: String| Loadout::new(name).with_carry("z".repeat(60 * 1024), 1);
    let row_bytes = files::row_bytes(&wide("wide000".into())).unwrap();
    let fit = (files::MAX_LOADOUTS_BYTES as usize - 2) / row_bytes;
    let mut form = script::LoadoutsStore::at(path.clone());
    form.restore(Vec::new());
    for n in 0..fit {
        form.upsert(wide(format!("wide{n:03}")));
    }
    form.save().unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().len() as usize,
        2 + fit * row_bytes,
        "the file is exactly its rows' share plus the brackets"
    );
    let version = service
        .readback_lease(&loadouts_identity(), WAIT)
        .unwrap()
        .unwrap()
        .version();
    assert_eq!(
        service.submit(loadout_edit(Some(version), wide("wide999".into()))),
        SubmitReceipt::Refused(ErrorCode::Validation {
            domain: LocalStateDomain::LoadoutsReferences,
            reason: ValidationReason::TooLong,
        })
    );
}

/// r2 item 8 (sol P1): one account's settlement is not displaced by other
/// accounts' settlements before its slot reads it.
#[test]
fn a_settlement_survives_other_accounts_settlements() {
    let scratch = IsolatedEnv::enter("local-state-retention");
    let service = service(&scratch, &[LocalStateDomain::BankHints]);
    let alice = bank_identity("alice");
    let bob = bank_identity("bob");
    assert_eq!(service.readback_lease(&alice, WAIT), Ok(None));
    assert_eq!(service.readback_lease(&bob, WAIT), Ok(None));
    cheap_files(&service);
    let alice_request = request(service.submit(hint_edit(&alice, None, vec![(995, 1)], 1)));
    completed(&service, alice_request);
    for n in 0..300 {
        let other = request(service.submit(hint_edit(&bob, None, vec![(995, n + 1)], 2)));
        completed(&service, other);
    }
    assert!(
        matches!(
            service.observe(alice_request),
            Some(Settlement::Completed(_))
        ),
        "alice's settlement was displaced by bob's"
    );
}

/// r2 item 8: a settlement the service no longer holds reads `Forgotten`,
/// never pending forever.
#[test]
fn a_settlement_no_longer_held_reads_forgotten() {
    let scratch = IsolatedEnv::enter("local-state-forgotten");
    let service = service(&scratch, &[LocalStateDomain::BankHints]);
    let alice = bank_identity("alice");
    assert_eq!(service.readback_lease(&alice, WAIT), Ok(None));
    cheap_files(&service);
    let old = request(service.submit(hint_edit(&alice, None, vec![(995, 1)], 1)));
    completed(&service, old);
    for n in 0..SETTLED_PER_STORE as i32 {
        let newer = request(service.submit(hint_edit(&alice, None, vec![(995, n + 2)], 2)));
        completed(&service, newer);
    }
    assert_eq!(service.request_state(old), RequestState::Forgotten);
    assert_eq!(service.observe(old), None);
}

/// Receipts are exact to their account, and versions come from one clock:
/// never reused, whichever account takes them.
#[test]
fn durable_receipts_are_exact_to_account_and_versions_are_never_reused() {
    let scratch = IsolatedEnv::enter("local-state-receipts");
    let service = service(&scratch, &[LocalStateDomain::BankHints]);
    let alice = bank_identity("alice");
    let bob = bank_identity("bob");
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    assert_eq!(service.readback_lease(&bob, WAIT).unwrap(), None);
    let alice_hydrated = service.lease(&alice).unwrap().version();
    let bob_hydrated = service.lease(&bob).unwrap().version();

    let alice_request = request(service.submit(hint_edit(&alice, None, vec![(995, 10)], 10)));
    let alice_receipt = completed(&service, alice_request);
    let bob_request = request(service.submit(hint_edit(&bob, None, vec![(995, 20)], 20)));
    let bob_receipt = completed(&service, bob_request);

    let versions = [
        alice_hydrated,
        bob_hydrated,
        alice_receipt.version(),
        bob_receipt.version(),
    ];
    assert!(
        versions.windows(2).all(|pair| pair[0] < pair[1]),
        "{versions:?}"
    );
    assert!(alice_receipt.accepts(&alice));
    assert!(!alice_receipt.accepts(&bob));
    assert!(bob_receipt.accepts(&bob));
    assert!(!bob_receipt.accepts(&alice));

    let audit = AuditCommit::after_commit(
        alice_receipt.identity().clone(),
        CommitAction::BankHintWrite,
        alice_hydrated,
        alice_receipt.version(),
        AuditOrigin::Submitted {
            request: alice_request,
        },
        alice_receipt.at(),
    );
    assert_eq!(audit.previous(), alice_hydrated);
    assert_eq!(audit.version(), alice_receipt.version());
    assert_eq!(audit.action(), CommitAction::BankHintWrite);
    assert_eq!(
        audit.origin(),
        AuditOrigin::Submitted {
            request: alice_request
        }
    );
}

/// The composition's Start-time lease: what `ScriptStartHandle` posts as
/// the operator's rows. A row the frontend's form saved after the service
/// hydrated is seen (the readback), the slot's capability carries the
/// account the server would log in, and a name the server would refuse
/// gets neither rows nor a capability.
#[test]
fn composition_leases_the_form_edited_loadouts_at_start_and_refuses_non_login_names() {
    let scratch = IsolatedEnv::enter("local-state-composition");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.loadouts_file();
    let service = LocalStateService::open(
        root,
        &[
            LocalStateDomain::LoadoutsReferences,
            LocalStateDomain::BankHints,
        ],
        Arc::new(RequestIdMinter::new()),
    );
    let composition = StateComposition::new(
        service,
        PROFILE,
        ContentScope::try_new(client::io::ClientRevision::R289, CONTENT).unwrap(),
    );
    // Hydrated with no file: a Start posts no rows.
    assert_eq!(
        composition.operator_loadouts("Alice Smith").unwrap(),
        Vec::new()
    );

    // The frontend's form writes the file directly (T5.5 moves it to the
    // port); the next Start's readback sees the row.
    let melee = Loadout::new("melee").with_slot("righthand", "Rune scimitar");
    let mut form = script::LoadoutsStore::at(path);
    form.upsert(melee.clone());
    form.save().unwrap();
    assert_eq!(
        composition.operator_loadouts("Alice Smith").unwrap(),
        vec![melee.clone()]
    );

    // The slot's capability leases the same rows under the login identity.
    let state = composition.slot_state("Alice Smith").expect("a login name");
    assert_eq!(state.account().profile(), PROFILE);
    assert_eq!(state.account().account(), "alice_smith");
    let leased = state.operator_loadouts().unwrap();
    assert_eq!(leased.rows(), std::slice::from_ref(&melee));
    assert_eq!(
        leased.version(),
        composition
            .service()
            .lease(&state.identity(LocalStateDomain::LoadoutsReferences))
            .unwrap()
            .version()
    );

    // A name the server refuses at login is refused here, not keyed.
    assert!(composition.slot_state("ab3456789xyz1").is_none());
    assert!(composition
        .operator_loadouts("ab3456789xyz1")
        .unwrap_err()
        .contains("login identity"));
    assert!(composition
        .identity(LocalStateDomain::BankHints, "a/b")
        .is_err());
}
